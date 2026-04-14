//! Persistent bootstrap/setup state for first-run setup workflows.

use crate::hotfix;
use anyhow::{Context, Result};
use lqos_config::{Config, UserRole};
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{AddressFamily, SockaddrLike},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const BOOTSTRAP_STATE_VERSION: u32 = 1;
const DEFAULT_LQOS_DIRECTORY: &str = "/opt/libreqos/src";
const DEFAULT_STATE_DIRECTORY: &str = "/opt/libreqos/state";
const SETUP_PORT: u16 = 9123;
const TOKEN_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// High-level setup status visible to operators.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupStatus {
    /// Setup is not complete yet.
    Incomplete,
    /// Setup completed, but subscriber topology inputs are still missing.
    CompleteWaitingForSubscriberData,
    /// Setup completed and subscriber topology inputs exist.
    CompleteShapingActive,
}

impl SetupStatus {
    /// Human-readable label for operator-facing status output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "Setup Incomplete",
            Self::CompleteWaitingForSubscriberData => {
                "Setup Complete, Waiting for Subscriber Data"
            }
            Self::CompleteShapingActive => "Setup Complete, Shaping Active",
        }
    }
}

/// Current bootstrap token metadata for the temporary setup surface.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapToken {
    /// Opaque setup token.
    pub token: String,
    /// Expiration time in Unix seconds.
    pub expires_at_unix: u64,
}

/// Persistent first-run setup state stored under the LibreQoS state directory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapState {
    /// On-disk schema version for setup state.
    pub version: u32,
    /// Whether setup has been completed.
    pub setup_complete: bool,
    /// Whether runtime subscriber inputs exist and shaping can become active.
    pub shaping_ready: bool,
    /// Whether an admin user exists.
    pub first_admin_exists: bool,
    /// Current setup token.
    pub token: BootstrapToken,
}

impl Default for BootstrapState {
    fn default() -> Self {
        Self {
            version: BOOTSTRAP_STATE_VERSION,
            setup_complete: false,
            shaping_ready: false,
            first_admin_exists: false,
            token: new_bootstrap_token(),
        }
    }
}

/// Computed setup status snapshot for CLI surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSnapshot {
    /// Synced bootstrap state.
    pub state: BootstrapState,
    /// Operator-visible status derived for the current snapshot.
    pub display_status: SetupStatus,
    /// Hostname and IP hints for the operator.
    pub host_hints: Vec<String>,
    /// Whether `network.json` exists.
    pub network_json_present: bool,
    /// Whether `ShapedDevices.csv` exists.
    pub shaped_devices_present: bool,
    /// Whether `/etc/lqos.conf` currently parses.
    pub config_loads: bool,
    /// Whether the Noble systemd hotfix is still required.
    pub hotfix_required: bool,
    /// Operator-visible hotfix status detail.
    pub hotfix_detail: String,
}

#[derive(Default, Deserialize)]
struct AuthUsersFile {
    #[serde(default)]
    users: Vec<AuthUserEntry>,
}

#[derive(Deserialize)]
struct AuthUserEntry {
    role: UserRole,
}

#[derive(Clone, Debug)]
struct RuntimePaths {
    lqos_directory: PathBuf,
    state_directory: PathBuf,
}

#[derive(Clone, Debug)]
struct LiveInputs {
    config_loads: bool,
    network_json_present: bool,
    shaped_devices_present: bool,
    first_admin_exists: bool,
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_bootstrap_token() -> BootstrapToken {
    BootstrapToken {
        token: Uuid::new_v4().simple().to_string(),
        expires_at_unix: current_unix_seconds() + TOKEN_TTL.as_secs(),
    }
}

fn runtime_paths() -> RuntimePaths {
    if let Ok(config) = lqos_config::load_config() {
        return RuntimePaths {
            lqos_directory: PathBuf::from(&config.lqos_directory),
            state_directory: config.resolved_state_directory(),
        };
    }

    RuntimePaths {
        lqos_directory: PathBuf::from(DEFAULT_LQOS_DIRECTORY),
        state_directory: PathBuf::from(DEFAULT_STATE_DIRECTORY),
    }
}

/// Returns the active or default LibreQoS runtime directory used by first-run setup.
pub fn runtime_lqos_directory() -> PathBuf {
    runtime_paths().lqos_directory
}

fn bootstrap_state_path() -> PathBuf {
    runtime_paths()
        .state_directory
        .join("setup")
        .join("bootstrap_state.json")
}

fn completion_report_path() -> PathBuf {
    runtime_paths()
        .state_directory
        .join("setup")
        .join("completion_report.txt")
}

fn auth_file_has_admin(lqos_directory: &Path) -> bool {
    let path = lqos_directory.join("lqusers.toml");
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(users) = toml::from_str::<AuthUsersFile>(&raw) else {
        return false;
    };
    users.users.iter().any(|user| user.role == UserRole::Admin)
}

fn collect_live_inputs() -> LiveInputs {
    let paths = runtime_paths();
    LiveInputs {
        config_loads: lqos_config::load_config().is_ok(),
        network_json_present: paths.lqos_directory.join("network.json").exists(),
        shaped_devices_present: paths.lqos_directory.join("ShapedDevices.csv").exists(),
        first_admin_exists: auth_file_has_admin(&paths.lqos_directory),
    }
}

fn display_status(setup_complete: bool, shaping_ready: bool) -> SetupStatus {
    if !setup_complete {
        SetupStatus::Incomplete
    } else if shaping_ready {
        SetupStatus::CompleteShapingActive
    } else {
        SetupStatus::CompleteWaitingForSubscriberData
    }
}

fn detect_host_hints() -> Vec<String> {
    let mut hints = BTreeSet::new();

    if let Ok(hostname) = fs::read_to_string("/etc/hostname") {
        let hostname = hostname.trim();
        if !hostname.is_empty() {
            hints.insert(hostname.to_string());
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
            if address.family() != Some(AddressFamily::Inet) {
                continue;
            }
            let Some(inet) = address.as_sockaddr_in() else {
                continue;
            };
            let ip = inet.ip();
            if !ip.is_loopback() {
                hints.insert(ip.to_string());
            }
        }
    }

    if hints.is_empty() {
        hints.insert("127.0.0.1".to_string());
    }

    hints.into_iter().collect()
}

fn detect_setup_hosts() -> Vec<String> {
    let hosts = detect_host_hints()
        .into_iter()
        .filter(|hint| hint.parse::<std::net::IpAddr>().is_ok())
        .collect::<Vec<_>>();
    if hosts.is_empty() {
        vec!["127.0.0.1".to_string()]
    } else {
        hosts
    }
}

fn build_setup_urls(token: &BootstrapToken) -> Vec<String> {
    detect_setup_hosts()
        .iter()
        .map(|host| format!("http://{host}:{SETUP_PORT}/setup?token={}", token.token))
        .collect()
}

fn hydrate_state(
    mut state: BootstrapState,
    refresh_expired_token: bool,
) -> BootstrapState {
    if refresh_expired_token && state.token.expires_at_unix <= current_unix_seconds() {
        state.token = new_bootstrap_token();
    }
    state.version = BOOTSTRAP_STATE_VERSION;
    state
}

fn read_persisted_state() -> Result<BootstrapState> {
    let path = bootstrap_state_path();
    if !path.exists() {
        return Ok(BootstrapState::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read bootstrap state {}", path.display()))?;
    serde_json::from_str::<BootstrapState>(&raw)
        .with_context(|| format!("Unable to parse bootstrap state {}", path.display()))
}

fn bootstrap_state_from_live(live: &LiveInputs) -> BootstrapState {
    BootstrapState {
        setup_complete: live.config_loads && live.first_admin_exists,
        shaping_ready: live.config_loads
            && live.first_admin_exists
            && live.network_json_present
            && live.shaped_devices_present,
        first_admin_exists: live.first_admin_exists,
        ..BootstrapState::default()
    }
}

/// Loads persisted bootstrap state, creating it if needed and syncing derived fields.
fn load_or_create_state() -> Result<BootstrapState> {
    let path = bootstrap_state_path();
    let live = collect_live_inputs();
    let loaded = if path.exists() {
        read_persisted_state()?
    } else {
        bootstrap_state_from_live(&live)
    };
    let synced = hydrate_state(loaded, true);
    if let Some(parent) = path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        if path.exists() {
            return Err(err).with_context(|| {
                format!("Unable to create setup state directory {}", parent.display())
            });
        }
        return Ok(synced);
    }

    if let Err(err) = save_state(&synced)
        && path.exists()
    {
        return Err(err);
    }
    Ok(synced)
}

/// Persists bootstrap state to disk.
pub fn save_state(state: &BootstrapState) -> Result<()> {
    let path = bootstrap_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create setup state directory {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state).context("Unable to serialize bootstrap state")?;
    fs::write(&path, raw)
        .with_context(|| format!("Unable to write bootstrap state {}", path.display()))?;
    Ok(())
}

/// Stores the most recent setup completion report for the setup web UI.
pub fn store_setup_completion_report(report: &str) -> Result<()> {
    let path = completion_report_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Unable to create setup state directory {}", parent.display())
        })?;
    }
    fs::write(&path, report)
        .with_context(|| format!("Unable to write setup completion report {}", path.display()))?;
    Ok(())
}

/// Loads the most recent setup completion report for the setup web UI.
pub fn load_setup_completion_report() -> Result<String> {
    let path = completion_report_path();
    fs::read_to_string(&path)
        .with_context(|| format!("Unable to read setup completion report {}", path.display()))
}

/// Records that setup successfully committed configuration.
pub fn record_setup_success(config: &Config) -> Result<BootstrapState> {
    let mut state = load_or_create_state()?;
    let live = LiveInputs {
        config_loads: true,
        network_json_present: Path::new(&config.lqos_directory).join("network.json").exists(),
        shaped_devices_present: Path::new(&config.lqos_directory)
            .join("ShapedDevices.csv")
            .exists(),
        first_admin_exists: auth_file_has_admin(Path::new(&config.lqos_directory)),
    };
    state.setup_complete = live.first_admin_exists;
    state.shaping_ready = live.network_json_present && live.shaped_devices_present;
    state.first_admin_exists = live.first_admin_exists;
    state = hydrate_state(state, true);
    save_state(&state)?;
    Ok(state)
}

/// Returns true when an admin user exists for the current or default runtime directory.
pub fn first_admin_exists() -> bool {
    auth_file_has_admin(&runtime_paths().lqos_directory)
}

/// Builds a current status snapshot for operator-facing CLI output.
pub fn status_snapshot() -> Result<StatusSnapshot> {
    let live = collect_live_inputs();
    let hotfix_status = hotfix::status()?;
    let mut state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    state.first_admin_exists = live.first_admin_exists;
    state.shaping_ready =
        state.setup_complete && live.network_json_present && live.shaped_devices_present;
    let host_hints = detect_host_hints();
    let display_status = display_status(state.setup_complete, state.shaping_ready);

    Ok(StatusSnapshot {
        state,
        display_status,
        host_hints,
        network_json_present: live.network_json_present,
        shaped_devices_present: live.shaped_devices_present,
        config_loads: live.config_loads,
        hotfix_required: hotfix_status.required,
        hotfix_detail: hotfix_status.detail,
    })
}

/// Renders a text report for `lqos_setup status`.
pub fn render_status_report() -> Result<String> {
    let snapshot = status_snapshot()?;
    let mut report = format!("lqos_setup status\n\nStatus: {}", snapshot.display_status.as_str());
    report.push_str("\nSource: persisted setup state plus current runtime diagnostics");

    report.push_str(&format!(
        "\n- setup_complete: {}",
        yes_no(snapshot.state.setup_complete)
    ));
    report.push_str(&format!(
        "\n- shaping_ready: {}",
        yes_no(snapshot.state.shaping_ready)
    ));
    report.push_str(&format!(
        "\n- config_loads: {}",
        yes_no(snapshot.config_loads)
    ));
    report.push_str(&format!(
        "\n- first_admin_exists: {}",
        yes_no(snapshot.state.first_admin_exists)
    ));
    report.push_str(&format!(
        "\n- network.json present: {}",
        yes_no(snapshot.network_json_present)
    ));
    report.push_str(&format!(
        "\n- ShapedDevices.csv present: {}",
        yes_no(snapshot.shaped_devices_present)
    ));
    report.push_str(&format!(
        "\n- hotfix required: {}",
        yes_no(snapshot.hotfix_required)
    ));
    report.push_str(&format!(
        "\n- hotfix detail: {}",
        snapshot.hotfix_detail
    ));

    if !snapshot.state.setup_complete {
        report.push_str("\n\nHost hints:");
        for host in &snapshot.host_hints {
            report.push_str(&format!("\n- {host}"));
        }
        report.push_str("\n\nRun `lqos_setup print-link` to print the current tokenized setup URL.");
    }

    Ok(report)
}

/// Returns true when setup is complete enough for runtime services to start.
///
/// Side effects: none.
pub fn runtime_services_should_start() -> Result<bool> {
    let live = collect_live_inputs();
    let hotfix_status = hotfix::status()?;
    let state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    Ok(
        state.setup_complete
            && live.first_admin_exists
            && live.config_loads
            && !hotfix_status.required,
    )
}

/// Returns true when first-run setup has not yet been completed.
pub fn setup_is_incomplete() -> Result<bool> {
    let live = collect_live_inputs();
    let state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    Ok(!state.setup_complete || !live.first_admin_exists)
}

/// Returns the current tokenized setup URLs if setup is still incomplete.
///
/// Side effects: may create `bootstrap_state.json` or refresh an expired setup token so the
/// returned links are durable and immediately usable.
pub fn current_setup_urls() -> Result<Vec<String>> {
    let state = load_or_create_state()?;
    if state.setup_complete {
        return Ok(Vec::new());
    }
    Ok(build_setup_urls(&state.token))
}

/// Validates a setup token against the persisted bootstrap state.
///
/// Side effects: none.
pub fn validate_setup_token(token: &str) -> Result<bool> {
    let state = if bootstrap_state_path().exists() {
        read_persisted_state()?
    } else {
        return Ok(false);
    };
    if state.setup_complete {
        return Ok(false);
    }
    Ok(state.token.expires_at_unix > current_unix_seconds() && state.token.token == token)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_TTL, current_unix_seconds, new_bootstrap_token};

    #[test]
    fn bootstrap_tokens_expire_in_future() {
        let now = current_unix_seconds();
        let token = new_bootstrap_token();
        assert!(!token.token.is_empty());
        assert!(token.expires_at_unix >= now + TOKEN_TTL.as_secs() - 1);
    }
}
