//! Optional Caddy-based HTTPS management shared by `lqosd` and `lqos_setup`.

use anyhow::{Context, Result, bail};
use default_net::get_default_interface;
use lqos_config::{Config, SslConfig, normalize_external_hostname};
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{AddressFamily, SockaddrLike},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CADDYFILE_PATH: &str = "/etc/caddy/Caddyfile";
const INTERNAL_CA_CERT_PATH: &str =
    "/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt";
const INSTALL_CADDY_SCRIPT: &str = "install_caddy.sh";
const DISABLE_CADDY_SCRIPT: &str = "disable_caddy.sh";
const RUNTIME_DIRECT_LISTEN: &str = ":::9123";
const RUNTIME_SECURE_LISTEN: &str = "127.0.0.1:9123";
const API_UPSTREAM: &str = "127.0.0.1:9122";
const WEB_UPSTREAM: &str = "127.0.0.1:9123";
const DELAYED_SWITCH_SHELL: &str = "/bin/sh";

/// The effective HTTPS mode for the current or requested config.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SslStatus {
    /// Whether SSL mode is enabled in the LibreQoS config.
    pub enabled: bool,
    /// Whether LibreQoS owns the generated Caddy configuration.
    pub managed_by_libreqos: bool,
    /// Optional public hostname used for trusted public certificates.
    pub external_hostname: Option<String>,
    /// Whether this config uses Caddy's internal certificate authority.
    pub using_internal_ca: bool,
    /// The operator-facing HTTPS URL for the current or requested state.
    pub target_url: String,
    /// The current WebUI listen address stored in the LibreQoS config.
    pub webserver_listen: Option<String>,
    /// Whether the Caddy binary is currently available on the host.
    pub caddy_installed: bool,
    /// Whether the managed Caddyfile currently exists on disk.
    pub caddyfile_present: bool,
    /// The local root certificate path used by internal-CA mode.
    pub internal_ca_root_certificate: Option<String>,
}

/// Outcome returned to operator-facing callers after an SSL action is queued or applied.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SslActionOutcome {
    /// Human-readable status message.
    pub message: String,
    /// The operator-facing URL to open after the action completes.
    pub target_url: String,
    /// Whether this action uses Caddy's internal certificate authority.
    pub using_internal_ca: bool,
    /// The local root certificate path used by internal-CA mode.
    pub internal_ca_root_certificate: Option<String>,
}

#[derive(Clone, Debug)]
struct SslRuntimePlan {
    external_hostname: Option<String>,
    target_url: String,
    using_internal_ca: bool,
    caddy_site_address: String,
}

/// Builds an operator-facing SSL status view from the current config and host state.
pub fn ssl_status(config: &Config, preferred_host: Option<&str>) -> SslStatus {
    let ssl = config.ssl.clone().unwrap_or_default();
    let normalized_hostname = normalized_config_hostname(ssl.external_hostname.as_deref());
    let plan = build_runtime_plan(normalized_hostname.clone(), preferred_host);
    SslStatus {
        enabled: ssl.enabled,
        managed_by_libreqos: ssl.managed_by_libreqos,
        external_hostname: normalized_hostname,
        using_internal_ca: plan.using_internal_ca,
        target_url: if ssl.enabled {
            plan.target_url
        } else {
            direct_runtime_url(config, preferred_host)
        },
        webserver_listen: config.webserver_listen.clone(),
        caddy_installed: caddy_binary_available(),
        caddyfile_present: Path::new(CADDYFILE_PATH).exists(),
        internal_ca_root_certificate: plan
            .using_internal_ca
            .then(|| INTERNAL_CA_CERT_PATH.to_string()),
    }
}

/// Enables HTTPS for the existing runtime install and schedules the live WebUI handoff.
///
/// Side effects:
/// - updates `/etc/lqos.conf`
/// - runs the Caddy installer helper if needed
/// - writes `/etc/caddy/Caddyfile`
/// - spawns a delayed background shell that restarts `caddy.service` and `lqosd.service`
pub fn enable_runtime_ssl(
    external_hostname: Option<String>,
    preferred_host: Option<&str>,
) -> Result<SslActionOutcome> {
    let existing = (*lqos_config::load_config()?).clone();
    let mut updated = existing.clone();
    let plan = apply_ssl_to_config(&mut updated, external_hostname, preferred_host)?;
    let caddyfile_backup = read_existing_caddyfile()?;
    let caddy_service_state = capture_service_state("caddy.service");
    persist_with_rollback(&updated)?;
    if let Err(err) = install_caddy(&updated)
        .and_then(|_| write_managed_caddyfile(&updated, preferred_host))
        .and_then(|_| schedule_delayed_runtime_switch(enable_runtime_command()))
    {
        let _ = lqos_config::update_config(&existing);
        let _ = restore_caddyfile(&caddyfile_backup);
        let _ = restore_service_state("caddy.service", caddy_service_state);
        bail!("{err:#}");
    }

    Ok(SslActionOutcome {
        message: if plan.using_internal_ca {
            format!(
                "Queued HTTPS setup with Caddy. LibreQoS will switch to {} and use Caddy's local certificate authority.",
                plan.target_url
            )
        } else {
            format!(
                "Queued HTTPS setup with Caddy. LibreQoS will switch to {} and request a trusted public certificate.",
                plan.target_url
            )
        },
        target_url: plan.target_url,
        using_internal_ca: plan.using_internal_ca,
        internal_ca_root_certificate: plan
            .using_internal_ca
            .then(|| INTERNAL_CA_CERT_PATH.to_string()),
    })
}

/// Disables HTTPS for the existing runtime install and schedules the direct WebUI handoff.
///
/// Side effects:
/// - updates `/etc/lqos.conf`
/// - removes the managed `/etc/caddy/Caddyfile`
/// - runs the Caddy disable helper when LibreQoS owns that setup
/// - spawns a delayed background shell that restarts `lqosd.service`
pub fn disable_runtime_ssl(preferred_host: Option<&str>) -> Result<SslActionOutcome> {
    let existing = (*lqos_config::load_config()?).clone();
    let ssl = existing
        .ssl
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("LibreQoS is not managing HTTPS on this node."))?;
    if !ssl.managed_by_libreqos {
        bail!("LibreQoS is not managing HTTPS on this node.");
    }
    let mut updated = existing.clone();
    let caddyfile_backup = read_existing_caddyfile()?;
    let caddy_service_state = capture_service_state("caddy.service");
    disable_ssl_in_config(&mut updated);
    persist_with_rollback(&updated)?;
    if let Err(err) = run_disable_caddy_script(&updated)
        .and_then(|_| schedule_delayed_runtime_switch(disable_runtime_command()))
    {
        let _ = lqos_config::update_config(&existing);
        let _ = restore_caddyfile(&caddyfile_backup);
        let _ = restore_service_state("caddy.service", caddy_service_state);
        bail!("{err:#}");
    }

    Ok(SslActionOutcome {
        message: format!(
            "Queued HTTPS shutdown. LibreQoS will return to direct access at {}.",
            direct_runtime_url(&updated, preferred_host)
        ),
        target_url: direct_runtime_url(&updated, preferred_host),
        using_internal_ca: false,
        internal_ca_root_certificate: None,
    })
}

/// Enables HTTPS in a setup flow before runtime services take over.
///
/// Side effects:
/// - updates `/etc/lqos.conf`
/// - runs the Caddy installer helper if needed
/// - writes `/etc/caddy/Caddyfile`
/// - enables and restarts `caddy.service` immediately
pub fn enable_setup_ssl(
    config: &Config,
    external_hostname: Option<String>,
    preferred_host: Option<&str>,
) -> Result<SslActionOutcome> {
    let existing = config.clone();
    let mut updated = config.clone();
    let plan = apply_ssl_to_config(&mut updated, external_hostname, preferred_host)?;
    let caddyfile_backup = read_existing_caddyfile()?;
    let caddy_service_state = capture_service_state("caddy.service");
    lqos_config::update_config(&updated)?;
    if let Err(err) = install_caddy(&updated)
        .and_then(|_| write_managed_caddyfile(&updated, preferred_host))
        .and_then(|_| {
            run_shell_command_now(enable_setup_command())
                .context("Unable to activate Caddy for setup")
        })
    {
        let _ = lqos_config::update_config(&existing);
        let _ = restore_caddyfile(&caddyfile_backup);
        let _ = restore_service_state("caddy.service", caddy_service_state);
        bail!("{err:#}");
    }

    Ok(SslActionOutcome {
        message: if plan.using_internal_ca {
            format!(
                "Configured optional HTTPS via Caddy. After runtime services start, open {} and trust the local root certificate on operator workstations.",
                plan.target_url
            )
        } else {
            format!(
                "Configured optional HTTPS via Caddy. After runtime services start, open {}.",
                plan.target_url
            )
        },
        target_url: plan.target_url,
        using_internal_ca: plan.using_internal_ca,
        internal_ca_root_certificate: plan
            .using_internal_ca
            .then(|| INTERNAL_CA_CERT_PATH.to_string()),
    })
}

fn persist_with_rollback(updated: &Config) -> Result<()> {
    updated.validate().map_err(anyhow::Error::msg)?;
    lqos_config::update_config(updated).with_context(|| "Unable to persist LibreQoS config")?;
    Ok(())
}

fn apply_ssl_to_config(
    config: &mut Config,
    external_hostname: Option<String>,
    preferred_host: Option<&str>,
) -> Result<SslRuntimePlan> {
    if config.disable_webserver.unwrap_or(false) {
        bail!("Disable Web Server is enabled. Re-enable the WebUI before setting up SSL.");
    }

    let plan = build_runtime_plan(
        normalize_external_hostname(external_hostname.as_deref().unwrap_or(""))
            .map_err(anyhow::Error::msg)?,
        preferred_host,
    );
    let previous_webserver_listen = config
        .webserver_listen
        .as_ref()
        .filter(|listen| listen.as_str() != RUNTIME_SECURE_LISTEN)
        .cloned()
        .or_else(|| {
            config
                .ssl
                .as_ref()
                .and_then(|ssl| ssl.previous_webserver_listen.clone())
        })
        .or_else(|| Some(RUNTIME_DIRECT_LISTEN.to_string()));
    config.webserver_listen = Some(RUNTIME_SECURE_LISTEN.to_string());
    config.ssl = Some(SslConfig {
        enabled: true,
        external_hostname: plan.external_hostname.clone(),
        managed_by_libreqos: true,
        previous_webserver_listen,
    });
    config.validate().map_err(anyhow::Error::msg)?;
    Ok(plan)
}

fn disable_ssl_in_config(config: &mut Config) {
    let mut restore_listen = Some(RUNTIME_DIRECT_LISTEN.to_string());
    if let Some(ssl) = config.ssl.as_mut() {
        ssl.enabled = false;
        restore_listen = ssl
            .previous_webserver_listen
            .clone()
            .filter(|listen| !listen.trim().is_empty())
            .or(restore_listen);
    }
    config.webserver_listen = restore_listen;
}

fn build_runtime_plan(
    external_hostname: Option<String>,
    preferred_host: Option<&str>,
) -> SslRuntimePlan {
    if let Some(hostname) = external_hostname {
        let target_url = format!("https://{hostname}/");
        return SslRuntimePlan {
            external_hostname: Some(hostname.clone()),
            target_url,
            using_internal_ca: false,
            caddy_site_address: hostname,
        };
    }

    let access_host = select_internal_access_host(preferred_host);
    SslRuntimePlan {
        external_hostname: None,
        target_url: format!("https://{}/", url_host(&access_host)),
        using_internal_ca: true,
        caddy_site_address: format!("https://{}", url_host(&access_host)),
    }
}

fn direct_runtime_url(config: &Config, preferred_host: Option<&str>) -> String {
    let access_host = normalize_preferred_host(preferred_host)
        .unwrap_or_else(|| detect_management_ip().to_string());
    let port = config
        .webserver_listen
        .as_deref()
        .and_then(listen_port)
        .unwrap_or(9123);
    format!("http://{}:{port}/", url_host(&access_host))
}

fn detect_management_ip() -> IpAddr {
    if let Ok(interface) = get_default_interface() {
        if let Some(ip) = interface
            .ipv4
            .into_iter()
            .map(|network| network.addr)
            .find(|ip| !ip.is_loopback())
        {
            return IpAddr::V4(ip);
        }
        if let Some(ip) = interface
            .ipv6
            .into_iter()
            .map(|network| network.addr)
            .find(|ip| !ip.is_loopback() && !ip.is_unspecified())
        {
            return IpAddr::V6(ip);
        }
    }

    if let Ok(ifaddrs) = getifaddrs() {
        for iface in ifaddrs {
            if iface.interface_name == "lo" {
                continue;
            }
            let Some(address) = iface.address else {
                continue;
            };
            match address.family() {
                Some(AddressFamily::Inet) => {
                    let Some(inet) = address.as_sockaddr_in() else {
                        continue;
                    };
                    let ip = inet.ip();
                    if !ip.is_loopback() {
                        return IpAddr::V4(ip);
                    }
                }
                Some(AddressFamily::Inet6) => {
                    let Some(inet) = address.as_sockaddr_in6() else {
                        continue;
                    };
                    let ip = inet.ip();
                    if !ip.is_loopback() && !ip.is_unspecified() {
                        return IpAddr::V6(ip);
                    }
                }
                _ => continue,
            }
        }
    }
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

fn select_internal_access_host(preferred_host: Option<&str>) -> String {
    normalize_preferred_host(preferred_host).unwrap_or_else(|| detect_management_ip().to_string())
}

fn normalized_config_hostname(hostname: Option<&str>) -> Option<String> {
    hostname.and_then(|hostname| normalize_external_hostname(hostname).ok().flatten())
}

fn normalize_preferred_host(preferred_host: Option<&str>) -> Option<String> {
    let host = preferred_host?.trim();
    if host.is_empty() {
        return None;
    }
    if let Some(stripped) = host.strip_prefix('[') {
        if let Some((ipv6, _port)) = stripped.split_once("]:") {
            return sanitize_candidate_host(ipv6);
        }
        if let Some(ipv6) = stripped.strip_suffix(']') {
            return sanitize_candidate_host(ipv6);
        }
    }
    if let Some((name, port)) = host.rsplit_once(':')
        && port.chars().all(|c| c.is_ascii_digit())
        && !name.contains(':')
    {
        return sanitize_candidate_host(name);
    }
    sanitize_candidate_host(host)
}

fn sanitize_candidate_host(host: &str) -> Option<String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    normalize_external_hostname(host).ok().flatten()
}

fn url_host(host: &str) -> String {
    host.parse::<IpAddr>()
        .ok()
        .and_then(|ip| match ip {
            IpAddr::V6(_) => Some(format!("[{host}]")),
            IpAddr::V4(_) => None,
        })
        .unwrap_or_else(|| host.to_string())
}

fn listen_port(listen: &str) -> Option<u16> {
    let port = listen.rsplit_once(':')?.1;
    port.parse::<u16>().ok()
}

#[derive(Clone, Copy)]
struct ServiceState {
    enabled: bool,
    active: bool,
}

fn capture_service_state(service_name: &str) -> ServiceState {
    ServiceState {
        enabled: command_succeeds("systemctl", &["is-enabled", "--quiet", service_name]),
        active: command_succeeds("systemctl", &["is-active", "--quiet", service_name]),
    }
}

fn restore_service_state(service_name: &str, state: ServiceState) -> Result<()> {
    if state.active {
        run_shell_command_now(&format!("systemctl restart {service_name}"))?;
    } else {
        run_shell_command_now(&format!("systemctl stop {service_name}"))?;
    }
    if state.enabled {
        run_shell_command_now(&format!("systemctl enable {service_name}"))?;
    } else {
        run_shell_command_now(&format!("systemctl disable {service_name}"))?;
    }
    Ok(())
}

fn command_succeeds(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn install_caddy(config: &Config) -> Result<()> {
    let script = runtime_script_path(config, INSTALL_CADDY_SCRIPT)?;
    let status = Command::new("/bin/bash")
        .arg(&script)
        .status()
        .with_context(|| format!("Unable to invoke {}", script.display()))?;
    if !status.success() {
        bail!("{} failed with exit status {status}", script.display());
    }
    Ok(())
}

fn run_disable_caddy_script(config: &Config) -> Result<()> {
    let script = runtime_script_path(config, DISABLE_CADDY_SCRIPT)?;
    let status = Command::new("/bin/bash")
        .arg(&script)
        .status()
        .with_context(|| format!("Unable to invoke {}", script.display()))?;
    if !status.success() {
        bail!("{} failed with exit status {status}", script.display());
    }
    Ok(())
}

fn runtime_script_path(config: &Config, script_name: &str) -> Result<PathBuf> {
    let path = Path::new(&config.lqos_directory).join(script_name);
    if !path.exists() {
        bail!(
            "Expected LibreQoS runtime helper {} to exist.",
            path.display()
        );
    }
    Ok(path)
}

fn write_managed_caddyfile(config: &Config, preferred_host: Option<&str>) -> Result<()> {
    let ssl = config
        .ssl
        .as_ref()
        .filter(|ssl| ssl.enabled)
        .ok_or_else(|| anyhow::anyhow!("SSL is not enabled in the LibreQoS config."))?;
    let plan = build_runtime_plan(ssl.external_hostname.clone(), preferred_host);
    let caddyfile = render_managed_caddyfile(&plan);
    let path = Path::new(CADDYFILE_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create {}", parent.display()))?;
    }
    fs::write(path, caddyfile).with_context(|| format!("Unable to write {CADDYFILE_PATH}"))?;
    Ok(())
}

fn read_existing_caddyfile() -> Result<Option<String>> {
    let path = Path::new(CADDYFILE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(path)
        .map(Some)
        .with_context(|| format!("Unable to read {CADDYFILE_PATH}"))
}

fn restore_caddyfile(contents: &Option<String>) -> Result<()> {
    let Some(contents) = contents else {
        return Ok(());
    };
    fs::write(CADDYFILE_PATH, contents)
        .with_context(|| format!("Unable to restore {CADDYFILE_PATH}"))
}

fn render_managed_caddyfile(plan: &SslRuntimePlan) -> String {
    let tls_block = if plan.using_internal_ca {
        "\n    tls internal"
    } else {
        ""
    };

    format!(
        "\
{{\n    admin off\n}}\n\n\
{site_address} {{{tls_block}\n    encode zstd gzip\n    handle /api/v1 {{\n        redir /api/v1/ 308\n    }}\n    handle_path /api/v1/* {{\n        reverse_proxy {api_upstream}\n    }}\n    reverse_proxy {web_upstream}\n}}\n",
        site_address = plan.caddy_site_address,
        tls_block = tls_block,
        api_upstream = API_UPSTREAM,
        web_upstream = WEB_UPSTREAM,
    )
}

fn enable_runtime_command() -> &'static str {
    "systemctl enable caddy.service && systemctl restart caddy.service && systemctl restart lqosd.service"
}

fn enable_setup_command() -> &'static str {
    "systemctl enable caddy.service && systemctl restart caddy.service"
}

fn disable_runtime_command() -> &'static str {
    "systemctl stop caddy.service || true; systemctl disable caddy.service || true; systemctl restart lqosd.service"
}

fn schedule_delayed_runtime_switch(command: &str) -> Result<()> {
    let mut child = Command::new(DELAYED_SWITCH_SHELL)
        .args(["-lc", &format!("sleep 2; {command}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Unable to spawn delayed runtime switch helper")?;
    let _ = child.stdin.take();
    Ok(())
}

fn run_shell_command_now(command: &str) -> Result<()> {
    let status = Command::new(DELAYED_SWITCH_SHELL)
        .args(["-lc", command])
        .status()
        .with_context(|| format!("Unable to invoke {DELAYED_SWITCH_SHELL}"))?;
    if !status.success() {
        bail!("Command failed with exit status {status}: {command}");
    }
    Ok(())
}

fn caddy_binary_available() -> bool {
    Command::new("caddy")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::{
        API_UPSTREAM, RUNTIME_DIRECT_LISTEN, RUNTIME_SECURE_LISTEN, WEB_UPSTREAM,
        apply_ssl_to_config, build_runtime_plan, disable_ssl_in_config, normalize_preferred_host,
        render_managed_caddyfile,
    };
    use lqos_config::{Config, normalize_external_hostname};

    #[test]
    fn blank_external_hostname_falls_back_to_internal_ca() {
        let plan = build_runtime_plan(normalize_external_hostname("   ").unwrap(), None);
        assert!(plan.using_internal_ca);
        assert!(plan.target_url.starts_with("https://"));
        assert!(plan.target_url.ends_with('/'));
    }

    #[test]
    fn managed_caddyfile_proxies_api_prefix_and_web_ui() {
        let plan = build_runtime_plan(Some("libreqos.example.com".to_string()), None);
        let caddyfile = render_managed_caddyfile(&plan);
        let expected = format!(
            "{{\n    admin off\n}}\n\nlibreqos.example.com {{\n    encode zstd gzip\n    handle /api/v1 {{\n        redir /api/v1/ 308\n    }}\n    handle_path /api/v1/* {{\n        reverse_proxy {API_UPSTREAM}\n    }}\n    reverse_proxy {WEB_UPSTREAM}\n}}\n"
        );
        assert_eq!(caddyfile, expected);
    }

    #[test]
    fn internal_ca_caddyfile_adds_tls_internal() {
        let plan = build_runtime_plan(None, None);
        let caddyfile = render_managed_caddyfile(&plan);
        assert!(caddyfile.contains("tls internal"));
    }

    #[test]
    fn external_hostname_rejects_scheme() {
        let error = normalize_external_hostname("https://libreqos.example.com")
            .expect_err("hostname with scheme should fail");
        assert!(error.to_string().contains("hostname only"));
    }

    #[test]
    fn malformed_preferred_host_is_rejected() {
        assert_eq!(normalize_preferred_host(Some("bad:port")), None);
        assert_eq!(normalize_preferred_host(Some("bad/path")), None);
    }

    #[test]
    fn internal_ca_plan_prefers_current_request_host() {
        let plan = build_runtime_plan(None, Some("setup.libreqos.test:9123"));
        assert_eq!(plan.target_url, "https://setup.libreqos.test/");
        assert_eq!(plan.caddy_site_address, "https://setup.libreqos.test");
    }

    #[test]
    fn enabling_ssl_preserves_existing_listener_for_disable() {
        let mut config = Config {
            webserver_listen: Some("192.0.2.50:9443".to_string()),
            ..Config::default()
        };
        let plan = apply_ssl_to_config(&mut config, Some("libreqos.example.com".to_string()), None)
            .expect("ssl config should apply");

        assert_eq!(plan.target_url, "https://libreqos.example.com/");
        assert_eq!(
            config.webserver_listen.as_deref(),
            Some(RUNTIME_SECURE_LISTEN)
        );
        assert_eq!(
            config
                .ssl
                .as_ref()
                .and_then(|ssl| ssl.previous_webserver_listen.as_deref()),
            Some("192.0.2.50:9443")
        );
    }

    #[test]
    fn enabling_ssl_captures_default_direct_listener_when_unset() {
        let mut config = Config::default();

        apply_ssl_to_config(&mut config, Some("libreqos.example.com".to_string()), None)
            .expect("ssl config should apply");

        assert_eq!(
            config
                .ssl
                .as_ref()
                .and_then(|ssl| ssl.previous_webserver_listen.as_deref()),
            Some(RUNTIME_DIRECT_LISTEN)
        );
    }

    #[test]
    fn disabling_ssl_restores_previous_listener() {
        let mut config = Config {
            webserver_listen: Some(RUNTIME_SECURE_LISTEN.to_string()),
            ..Config::default()
        };
        apply_ssl_to_config(&mut config, Some("libreqos.example.com".to_string()), None)
            .expect("ssl config should apply");
        config
            .ssl
            .as_mut()
            .expect("ssl config should exist")
            .previous_webserver_listen = Some("192.0.2.60:9000".to_string());

        disable_ssl_in_config(&mut config);

        assert_eq!(config.webserver_listen.as_deref(), Some("192.0.2.60:9000"));
        assert!(
            !config
                .ssl
                .as_ref()
                .expect("ssl config should still exist")
                .enabled
        );
    }
}
