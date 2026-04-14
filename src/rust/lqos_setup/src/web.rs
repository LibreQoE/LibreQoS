//! Minimal setup-only web server for first-run LibreQoS configuration.

use crate::{
    config_builder::{BridgeMode, CURRENT_CONFIG},
    interfaces,
    service_handoff,
    setup_actions::{self, CommitOutcome},
};
use anyhow::{Context, Result, bail};
use axum::{
    Form, Router,
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use lqos_config::{UserRole, WebUsers};
use lqos_setup::{bootstrap, hotfix};
use serde::Deserialize;
use std::fmt::Write as _;

const BIND_ADDR: &str = "0.0.0.0:9123";

#[derive(Debug, Deserialize)]
struct SetupQuery {
    token: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompletedQuery {
    auto: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminForm {
    token: String,
    username: String,
    password: String,
    confirm_password: String,
}

#[derive(Debug, Deserialize)]
struct SaveForm {
    token: String,
    node_name: String,
    downlink_mbps: String,
    uplink_mbps: String,
    bridge_mode: String,
    to_internet: String,
    to_network: Option<String>,
    single_interface: Option<String>,
    internet_vlan: Option<String>,
    network_vlan: Option<String>,
    allow_subnets: String,
}

#[derive(Debug, Deserialize)]
struct HotfixForm {
    token: String,
}

#[derive(Debug, Deserialize)]
struct PendingForm {
    token: String,
    operation_id: String,
}

pub(crate) fn run() -> Result<()> {
    if let Ok(urls) = bootstrap::current_setup_urls()
        && !urls.is_empty()
    {
        for url in urls {
            println!("Setup URL: {url}");
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Unable to create Tokio runtime for setup web server")?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(BIND_ADDR)
            .await
            .with_context(|| format!("Unable to bind setup web server to {BIND_ADDR}"))?;
        let app = Router::new()
            .route("/", get(root))
            .route("/setup", get(setup_page))
            .route("/setup/completed", get(completed_page))
            .route("/setup/install-hotfix", post(install_hotfix))
            .route("/setup/create-admin", post(create_admin))
            .route("/setup/save", post(save_setup))
            .route("/setup/confirm", get(root).post(confirm_setup))
            .route("/setup/revert", get(root).post(revert_setup));
        axum::serve(listener, app)
            .await
            .context("Setup web server exited unexpectedly")
    })
}

async fn root() -> Redirect {
    let target = match bootstrap::status_snapshot() {
        Ok(snapshot) if snapshot.state.setup_complete => "/setup/completed?auto=1",
        _ => "/setup",
    };
    Redirect::temporary(target)
}

async fn setup_page(Query(query): Query<SetupQuery>) -> Response {
    match render_setup_page(query.token.as_deref(), query.notice.as_deref(), None) {
        Ok(html) => Html(html).into_response(),
        Err(err) => error_response(StatusCode::FORBIDDEN, &err.to_string()),
    }
}

async fn completed_page(Query(query): Query<CompletedQuery>) -> Response {
    let automatic = query.auto.as_deref() == Some("1");
    match render_completed_page(automatic) {
        Ok(html) => Html(html).into_response(),
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
}

async fn create_admin(Form(form): Form<AdminForm>) -> Response {
    if let Err(err) = ensure_valid_setup_token(&form.token) {
        return error_response(StatusCode::FORBIDDEN, &err.to_string());
    }
    if form.password != form.confirm_password {
        return error_response(StatusCode::BAD_REQUEST, "Passwords did not match.");
    }
    if form.username.trim().is_empty() || form.password.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Username and password are required.");
    }

    match WebUsers::load_or_create_in(&bootstrap::runtime_lqos_directory()).and_then(|mut users| {
        users.add_or_update_user(form.username.trim(), &form.password, UserRole::Admin)
    }) {
        Ok(()) => match render_setup_page(Some(&form.token), Some("Admin account created."), None)
        {
            Ok(html) => Html(html).into_response(),
            Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
        },
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Unable to create admin user: {err}"),
        ),
    }
}

async fn install_hotfix(Form(form): Form<HotfixForm>) -> Response {
    if let Err(err) = ensure_valid_setup_token(&form.token) {
        return error_response(StatusCode::FORBIDDEN, &err.to_string());
    }
    match hotfix::install() {
        Ok(result) => {
            match render_setup_page(Some(&form.token), Some(&result.summary), Some(&result.detail))
            {
                Ok(html) => Html(html).into_response(),
                Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
            }
        }
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err.to_string()),
    }
}

async fn save_setup(Form(form): Form<SaveForm>) -> Response {
    if let Err(err) = ensure_valid_setup_token(&form.token) {
        return error_response(StatusCode::FORBIDDEN, &err.to_string());
    }
    match hotfix::status() {
        Ok(status) if status.required => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Install the Noble systemd hotfix before saving setup.",
            );
        }
        Ok(_) => {}
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
    if !bootstrap::first_admin_exists() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Create the first admin account before saving setup settings.",
        );
    }
    if let Err(err) = apply_form_to_current_config(&form) {
        return error_response(StatusCode::BAD_REQUEST, &err.to_string());
    }

    match setup_actions::prepare_commit() {
        Ok(CommitOutcome::Complete(success)) => match finalize_success_html(&success.config, success.event_log) {
            Ok(html) => Html(html).into_response(),
            Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
        },
        Ok(CommitOutcome::Pending(pending)) => {
            Html(render_pending_page(&form.token, &pending.operation_id, &pending.prompt))
                .into_response()
        }
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err.to_string()),
    }
}

async fn confirm_setup(Form(form): Form<PendingForm>) -> Response {
    if let Err(err) = ensure_valid_setup_token(&form.token) {
        return error_response(StatusCode::FORBIDDEN, &err.to_string());
    }
    match setup_actions::confirm_pending_commit(&form.operation_id) {
        Ok(success) => match finalize_success_html(&success.config, success.event_log) {
            Ok(html) => Html(html).into_response(),
            Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
        },
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err.to_string()),
    }
}

async fn revert_setup(Form(form): Form<PendingForm>) -> Response {
    if let Err(err) = ensure_valid_setup_token(&form.token) {
        return error_response(StatusCode::FORBIDDEN, &err.to_string());
    }
    match setup_actions::revert_pending_commit(&form.operation_id) {
        Ok(message) => match render_setup_page(Some(&form.token), Some(&message), None) {
            Ok(html) => Html(html).into_response(),
            Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
        },
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err.to_string()),
    }
}

fn render_setup_page(
    token: Option<&str>,
    notice: Option<&str>,
    notice_detail: Option<&str>,
) -> Result<String> {
    let snapshot = bootstrap::status_snapshot()?;
    let token = token.ok_or_else(|| anyhow::anyhow!("Setup requires a valid tokenized URL."))?;
    ensure_valid_setup_token(token)?;
    let interface_options = interfaces::get_interface_options().unwrap_or_default();
    let config = CURRENT_CONFIG.lock().clone();
    let mode_label = match config.bridge_mode {
        BridgeMode::Single => "single",
        _ => "linux",
    };

    let mut html = page_shell("LibreQoS Setup");
    html.push_str("<main class=\"setup-shell\">");
    html.push_str("<section class=\"card card-hero\"><div class=\"brand-row\"><span class=\"brand-badge\">LibreQoS</span><span class=\"brand-caption\">First-Run Setup</span></div><h1>LibreQoS Setup</h1>");
    html.push_str(&format!(
        "<p class=\"muted\">Status: <strong>{}</strong></p>",
        escape_html(snapshot.display_status.as_str())
    ));
    if let Some(notice) = notice {
        html.push_str("<div class=\"notice\">");
        let _ = write!(html, "<strong>{}</strong>", escape_html(notice));
        if let Some(detail) = notice_detail {
            let _ = write!(
                html,
                "<details class=\"notice-details\"><summary>Show details</summary><pre class=\"report\">{}</pre></details>",
                escape_html(detail)
            );
        }
        html.push_str("</div>");
    }
    html.push_str("<ul class=\"status-list\">");
    html.push_str(&format!(
        "<li>Admin user exists: {}</li>",
        yes_no(snapshot.state.first_admin_exists)
    ));
    html.push_str(&format!(
        "<li>Config loads: {}</li>",
        yes_no(snapshot.config_loads)
    ));
    html.push_str(&format!(
        "<li>network.json present: {}</li>",
        yes_no(snapshot.network_json_present)
    ));
    html.push_str(&format!(
        "<li>ShapedDevices.csv present: {}</li>",
        yes_no(snapshot.shaped_devices_present)
    ));
    html.push_str(&format!(
        "<li>Hotfix required: {}</li>",
        yes_no(snapshot.hotfix_required)
    ));
    html.push_str("</ul></section>");

    if snapshot.hotfix_required {
        html.push_str("<section class=\"card\"><h2>Ubuntu 24.04 Hotfix Required</h2>");
        html.push_str(&format!(
            "<p class=\"notice\">{}</p>",
            escape_html(&snapshot.hotfix_detail)
        ));
        html.push_str("<p class=\"muted\">This can take around 30 seconds while LibreQoS upgrades the required systemd packages.</p>");
        html.push_str("<form method=\"post\" action=\"/setup/install-hotfix\" onsubmit=\"return submitWithProgress(this, 'Installing hotfix... This page will refresh when the install finishes.');\">");
        hidden_token(&mut html, token);
        html.push_str("<button type=\"submit\">Install Hotfix</button></form></section>");
    }

    if !snapshot.state.first_admin_exists {
        html.push_str("<section class=\"card\"><h2>Create First Admin</h2>");
        html.push_str("<form method=\"post\" action=\"/setup/create-admin\" onsubmit=\"return submitWithProgress(this, 'Creating admin account...');\">");
        hidden_token(&mut html, token);
        html.push_str("<label>Username<input type=\"text\" name=\"username\" required></label>");
        html.push_str("<label>Password<input type=\"password\" name=\"password\" required></label>");
        html.push_str("<label>Confirm Password<input type=\"password\" name=\"confirm_password\" required></label>");
        html.push_str("<button type=\"submit\">Create Admin</button></form></section>");
    } else if !snapshot.hotfix_required {
        html.push_str("<section class=\"card\"><h2>Core Setup</h2>");
        html.push_str("<form id=\"coreSetupForm\" method=\"post\" action=\"/setup/save\" onsubmit=\"return validateAndSubmitSetup(this);\">");
        hidden_token(&mut html, token);
        html.push_str(&format!(
            "<label>Node Name<input type=\"text\" name=\"node_name\" value=\"{}\" required></label>",
            escape_html(&config.node_name)
        ));
        html.push_str(&format!(
            "<label>Downlink Mbps<input type=\"number\" min=\"1\" name=\"downlink_mbps\" value=\"{}\" required></label>",
            config.mbps_to_internet
        ));
        html.push_str(&format!(
            "<label>Uplink Mbps<input type=\"number\" min=\"1\" name=\"uplink_mbps\" value=\"{}\" required></label>",
            config.mbps_to_network
        ));
        html.push_str("<label>Mode<select name=\"bridge_mode\">");
        html.push_str(&format!(
            "<option value=\"linux\"{}>Linux Bridge</option>",
            selected(mode_label == "linux")
        ));
        html.push_str(&format!(
            "<option value=\"single\"{}>Single Interface</option>",
            selected(mode_label == "single")
        ));
        html.push_str("</select></label>");
        html.push_str("<div class=\"mode-section\" data-mode=\"linux\">");
        html.push_str("<p class=\"muted\">Recommended for most installs. Select the WAN-facing and subscriber-facing interfaces.</p>");
        html.push_str("<label>To Internet<select name=\"to_internet\">");
        for iface in &interface_options {
            html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape_html(&iface.name),
                selected(config.to_internet == iface.name),
                escape_html(&iface.label)
            ));
        }
        html.push_str("</select></label>");
        html.push_str("<label>To Network<select name=\"to_network\">");
        for iface in &interface_options {
            html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape_html(&iface.name),
                selected(config.to_network == iface.name),
                escape_html(&iface.label)
            ));
        }
        html.push_str("</select></label>");
        html.push_str("<p id=\"interfaceValidationMessage\" class=\"validation-message\" hidden>Internet and network interfaces must be different.</p>");
        html.push_str("</div>");
        html.push_str("<div class=\"mode-section\" data-mode=\"single\">");
        html.push_str("<p class=\"muted\">Use this when a single interface carries both internet and subscriber traffic with optional VLAN tags.</p>");
        html.push_str("<label>Interface<select name=\"single_interface\">");
        for iface in &interface_options {
            html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape_html(&iface.name),
                selected(config.to_internet == iface.name),
                escape_html(&iface.label)
            ));
        }
        html.push_str("</select></label>");
        html.push_str(&format!(
            "<label>Internet VLAN<input type=\"number\" min=\"0\" name=\"internet_vlan\" value=\"{}\"></label>",
            config.internet_vlan
        ));
        html.push_str(&format!(
            "<label>Network VLAN<input type=\"number\" min=\"0\" name=\"network_vlan\" value=\"{}\"></label>",
            config.network_vlan
        ));
        html.push_str("</div>");
        html.push_str(&format!(
            "<label>Allowed Subnets<textarea name=\"allow_subnets\" rows=\"6\">{}</textarea></label>",
            escape_html(&config.allow_subnets.join("\n"))
        ));
        html.push_str("<button id=\"saveSetupButton\" type=\"submit\">Save Setup</button></form></section>");
    }

    html.push_str("<section class=\"card\"><h2>Host Hints</h2><ul class=\"status-list\">");
    for host in snapshot.host_hints {
        html.push_str(&format!("<li>{}</li>", escape_html(&host)));
    }
    html.push_str("</ul><p class=\"muted\">Use <code>lqos_setup print-link</code> to mint or refresh the current setup link.</p></section>");
    html.push_str("<div id=\"busyOverlay\" class=\"busy-overlay\" aria-live=\"polite\" hidden><div class=\"busy-card\"><div class=\"busy-bar\"></div><p id=\"busyMessage\">Working...</p></div></div>");
    html.push_str("</main></body></html>");
    Ok(html)
}

fn apply_form_to_current_config(form: &SaveForm) -> Result<()> {
    let downlink = form
        .downlink_mbps
        .trim()
        .parse::<u64>()
        .context("Downlink bandwidth must be a positive integer.")?;
    let uplink = form
        .uplink_mbps
        .trim()
        .parse::<u64>()
        .context("Uplink bandwidth must be a positive integer.")?;
    let allow_subnets = form
        .allow_subnets
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut config = CURRENT_CONFIG.lock();
    config.node_name = form.node_name.trim().to_string();
    config.mbps_to_internet = downlink;
    config.mbps_to_network = uplink;
    config.allow_subnets = allow_subnets;
    match form.bridge_mode.as_str() {
        "single" => {
            config.bridge_mode = BridgeMode::Single;
            let interface = form
                .single_interface
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("A shaping interface is required."))?;
            config.to_internet = interface.to_string();
            config.to_network.clear();
            config.internet_vlan = parse_vlan(form.internet_vlan.as_deref())?;
            config.network_vlan = parse_vlan(form.network_vlan.as_deref())?;
        }
        "linux" => {
            let to_network = form
                .to_network
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("A network-facing interface is required."))?;
            if form.to_internet.trim() == to_network {
                bail!("Internet and network interfaces must be different.");
            }
            config.bridge_mode = BridgeMode::Linux;
            config.to_internet = form.to_internet.trim().to_string();
            config.to_network = to_network.to_string();
            config.internet_vlan = 0;
            config.network_vlan = 0;
        }
        _ => bail!("Unsupported bridge mode."),
    }
    Ok(())
}

fn parse_vlan(raw: Option<&str>) -> Result<u32> {
    let value = raw.unwrap_or("0").trim();
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse::<u32>()
        .with_context(|| format!("Invalid VLAN value: {value}"))
}

fn ensure_valid_setup_token(token: &str) -> Result<()> {
    if bootstrap::validate_setup_token(token)? {
        Ok(())
    } else {
        bail!("Setup token is missing, invalid, or expired. Run `lqos_setup print-link` for a current setup URL.");
    }
}

fn hidden_token(html: &mut String, token: &str) {
    let _ = write!(
        html,
        "<input type=\"hidden\" name=\"token\" value=\"{}\">",
        escape_html(token)
    );
}

fn render_pending_page(token: &str, operation_id: &str, prompt: &str) -> String {
    let mut html = page_shell("Confirm Network Change");
    html.push_str("<main class=\"setup-shell\"><section class=\"card\">");
    html.push_str("<h1>Confirm Network Change</h1>");
    html.push_str(&format!(
        "<pre class=\"report\">{}</pre>",
        escape_html(prompt)
    ));
    html.push_str("<div class=\"button-row\">");
    html.push_str("<form method=\"post\" action=\"/setup/confirm\" onsubmit=\"return submitWithProgress(this, 'Confirming managed network change...');\">");
    hidden_token(&mut html, token);
    let _ = write!(
        html,
        "<input type=\"hidden\" name=\"operation_id\" value=\"{}\">",
        escape_html(operation_id)
    );
    html.push_str("<button type=\"submit\">Confirm</button></form>");
    html.push_str("<form method=\"post\" action=\"/setup/revert\" onsubmit=\"return submitWithProgress(this, 'Reverting pending network change...');\">");
    hidden_token(&mut html, token);
    let _ = write!(
        html,
        "<input type=\"hidden\" name=\"operation_id\" value=\"{}\">",
        escape_html(operation_id)
    );
    html.push_str("<button type=\"submit\" class=\"secondary\">Revert</button></form>");
    html.push_str("</div><div id=\"busyOverlay\" class=\"busy-overlay\" aria-live=\"polite\" hidden><div class=\"busy-card\"><div class=\"busy-bar\"></div><p id=\"busyMessage\">Working...</p></div></div></section></main></body></html>");
    html
}

fn finalize_success_html(
    config: &lqos_config::Config,
    mut event_log: Vec<String>,
) -> Result<String> {
    setup_actions::persist_setup_success(config, &mut event_log)?;
    let handoff_notice = match service_handoff::schedule_runtime_handoff() {
        Ok(notice) => {
            event_log.push(notice.message.clone());
            Some(notice)
        }
        Err(err) => {
            event_log.push(format!("WARNING: {err:#}"));
            None
        }
    };
    let snapshot = bootstrap::status_snapshot()?;
    let report = event_log.join("\n");
    let _ = bootstrap::store_setup_completion_report(&report);
    let mut html = page_shell("Setup Complete");
    html.push_str("<main class=\"setup-shell\"><section class=\"card\"><h1>Setup Saved</h1>");
    html.push_str(&format!(
        "<p class=\"muted\">Current status: <strong>{}</strong></p>",
        escape_html(snapshot.display_status.as_str())
    ));
    html.push_str(&format!(
        "<details class=\"notice-details\" open><summary>Show setup actions</summary><pre class=\"report\">{}</pre></details>",
        escape_html(&report)
    ));
    if handoff_notice
        .as_ref()
        .is_some_and(|notice| notice.automatic)
    {
        html.push_str("<p class=\"muted\">This setup page may stop while LibreQoS runtime services start. Refresh this address after the handoff if you want to load the normal runtime UI.</p>");
        html.push_str(
            "<script>if(window.history&&window.history.replaceState){window.history.replaceState({},'', '/');}setTimeout(function(){window.location.replace('/');},1500);</script>",
        );
    }
    html.push_str("</section></main></body></html>");
    Ok(html)
}

fn render_completed_page(automatic: bool) -> Result<String> {
    let snapshot = bootstrap::status_snapshot()?;
    let report = bootstrap::load_setup_completion_report().unwrap_or_default();
    let mut html = page_shell("Setup Complete");
    html.push_str("<main class=\"setup-shell\"><section class=\"card\"><h1>Setup Saved</h1>");
    html.push_str(&format!(
        "<p class=\"muted\">Current status: <strong>{}</strong></p>",
        escape_html(snapshot.display_status.as_str())
    ));
    if !report.is_empty() {
        html.push_str(&format!(
            "<details class=\"notice-details\" open><summary>Show setup actions</summary><pre class=\"report\">{}</pre></details>",
            escape_html(&report)
        ));
    }
    if automatic {
        html.push_str("<p class=\"muted\">This setup page may stop while LibreQoS runtime services start. Refresh this address after the handoff if you want to load the normal runtime UI.</p>");
        html.push_str(
            "<script>setTimeout(function(){window.location.replace('/');},1500);</script>",
        );
    }
    html.push_str("</section></main></body></html>");
    Ok(html)
}

fn page_shell(title: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{}</style><script>{}</script></head><body onload=\"setupPage();\">",
        escape_html(title),
        base_css(),
        base_js()
    )
}

fn base_css() -> &'static str {
    ":root{--lqos-font-sans:\"InterVar\",system-ui,-apple-system,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial,sans-serif;--lqos-bg:radial-gradient(1200px 800px at 10% 0%,rgba(59,130,246,.14),transparent 60%),radial-gradient(900px 600px at 90% 20%,rgba(16,185,129,.12),transparent 55%),linear-gradient(180deg,#f7f8fb 0%,#eef4fb 100%);--lqos-surface:rgba(255,255,255,.82);--lqos-surface-2:rgba(248,250,252,.94);--lqos-border:rgba(15,23,42,.12);--lqos-shadow:0 18px 50px rgba(0,0,0,.10),0 6px 18px rgba(0,0,0,.06);--lqos-primary:#2563eb;--lqos-primary-dark:#1d4ed8;--lqos-accent:#0f766e;--lqos-text:#0f172a;--lqos-muted:#5b6471}\
    *{box-sizing:border-box}\
    body{margin:0;font-family:var(--lqos-font-sans);background:var(--lqos-bg);color:var(--lqos-text);min-height:100vh}\
    .setup-shell{max-width:900px;margin:0 auto;padding:32px 20px 64px}\
    .card{background:var(--lqos-surface);border:1px solid var(--lqos-border);border-radius:18px;padding:24px;margin-bottom:18px;box-shadow:var(--lqos-shadow);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px)}\
    .card-hero{background:linear-gradient(180deg,rgba(255,255,255,.94),rgba(255,255,255,.82));position:relative;overflow:hidden}\
    .card-hero::after{content:\"\";position:absolute;inset:auto -10% -30% auto;width:280px;height:280px;background:radial-gradient(circle,rgba(37,99,235,.16) 0%,rgba(15,118,110,.08) 45%,transparent 72%);pointer-events:none}\
    .brand-row{display:flex;align-items:center;gap:10px;margin-bottom:12px;flex-wrap:wrap}\
    .brand-badge{display:inline-flex;align-items:center;padding:6px 10px;border-radius:999px;background:linear-gradient(135deg,var(--lqos-primary),var(--lqos-accent));color:#fff;font-size:.78rem;font-weight:800;letter-spacing:.04em;text-transform:uppercase}\
    .brand-caption{font-size:.85rem;font-weight:700;color:var(--lqos-primary-dark);text-transform:uppercase;letter-spacing:.08em}\
    h1,h2{margin:0 0 14px}\
    h1{font-size:2rem;line-height:1.1}\
    label{display:block;font-weight:700;margin:14px 0 6px}\
    input,select,textarea{width:100%;box-sizing:border-box;border:1px solid rgba(15,23,42,.14);border-radius:12px;padding:10px 12px;font:inherit;background:rgba(255,255,255,.98);box-shadow:inset 0 1px 2px rgba(15,23,42,.04)}\
    input:focus,select:focus,textarea:focus{outline:none;border-color:rgba(37,99,235,.45);box-shadow:0 0 0 4px rgba(59,130,246,.16)}\
    button{margin-top:16px;background:linear-gradient(135deg,var(--lqos-primary),var(--lqos-accent));color:#fff;border:0;border-radius:999px;padding:11px 18px;font:inherit;font-weight:800;cursor:pointer;box-shadow:0 12px 24px rgba(37,99,235,.18)}\
    button[disabled]{opacity:.7;cursor:progress}\
    button.secondary{background:linear-gradient(135deg,#64748b,#475569)}.button-row{display:flex;gap:12px;flex-wrap:wrap}.button-row form{margin:0}\
    .muted{color:var(--lqos-muted)}.notice{background:#fff7d6;border:1px solid #e7d48b;padding:12px 14px;border-radius:12px}\
    .notice-details{margin-top:10px}.notice-details summary{cursor:pointer;font-weight:700}\
    .mode-section[hidden]{display:none}\
    .status-list{margin:0;padding-left:18px}.status-list li{margin:6px 0}.report{white-space:pre-wrap;background:var(--lqos-surface-2);border:1px solid #d5dde7;border-radius:12px;padding:16px;overflow:auto}\
    .busy-overlay{position:fixed;inset:0;background:rgba(15,23,42,.28);display:flex;align-items:center;justify-content:center;padding:20px;z-index:9999}\
    .busy-overlay[hidden]{display:none !important}\
    .busy-card{width:min(520px,100%);background:#fff;border-radius:16px;padding:24px;box-shadow:0 18px 40px rgba(15,23,42,.18)}\
    .busy-bar{height:12px;border-radius:999px;background:linear-gradient(90deg,var(--lqos-primary) 0%,#14b8a6 30%,#d1fae5 50%,#14b8a6 70%,var(--lqos-primary) 100%);background-size:200% 100%;animation:busy-slide 1.2s linear infinite;margin-bottom:14px}\
    @keyframes busy-slide{0%{background-position:200% 0}100%{background-position:-200% 0}}\
    .validation-message{margin:10px 0 0;color:#b91c1c;font-weight:700}\
    code{background:#eef2f7;padding:2px 6px;border-radius:6px}"
}

fn base_js() -> &'static str {
    "function syncModeSections(){const mode=document.querySelector('select[name=\"bridge_mode\"]')?.value||'linux';document.querySelectorAll('.mode-section').forEach((section)=>{section.hidden=section.dataset.mode!==mode;});updateCoreSetupValidation();}\
    function showBusy(message){const overlay=document.getElementById('busyOverlay');const target=document.getElementById('busyMessage');if(target&&message){target.textContent=message;}if(overlay){overlay.hidden=false;}}\
    function hideBusy(){const overlay=document.getElementById('busyOverlay');if(overlay){overlay.hidden=true;}}\
    function setSubmitDisabled(form,disabled){const button=form?.querySelector('button[type=\"submit\"]');if(button){button.disabled=!!disabled;}}\
    function submitWithProgress(form,message){setSubmitDisabled(form,true);showBusy(message);return true;}\
    function currentBridgeMode(){return document.querySelector('select[name=\"bridge_mode\"]')?.value||'linux';}\
    function linuxInterfacesAreDistinct(){const toInternet=document.querySelector('select[name=\"to_internet\"]')?.value||'';const toNetwork=document.querySelector('select[name=\"to_network\"]')?.value||'';return !toInternet||!toNetwork||toInternet!==toNetwork;}\
    function updateCoreSetupValidation(){const form=document.getElementById('coreSetupForm');if(!form){return true;}const message=document.getElementById('interfaceValidationMessage');const invalid=currentBridgeMode()==='linux'&&!linuxInterfacesAreDistinct();if(message){message.hidden=!invalid;}setSubmitDisabled(form,invalid);return !invalid;}\
    function validateAndSubmitSetup(form){if(!updateCoreSetupValidation()){hideBusy();setSubmitDisabled(form,false);return false;}return submitWithProgress(form,'Saving setup and applying managed network changes...');}\
    function resetBusyState(){hideBusy();document.querySelectorAll('form').forEach((form)=>setSubmitDisabled(form,false));updateCoreSetupValidation();}\
    function setupPage(){const mode=document.querySelector('select[name=\"bridge_mode\"]');if(mode){mode.addEventListener('change',syncModeSections);}const toInternet=document.querySelector('select[name=\"to_internet\"]');if(toInternet){toInternet.addEventListener('change',updateCoreSetupValidation);}const toNetwork=document.querySelector('select[name=\"to_network\"]');if(toNetwork){toNetwork.addEventListener('change',updateCoreSetupValidation);}window.addEventListener('pageshow',resetBusyState);syncModeSections();resetBusyState();}"
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let mut html = page_shell("Setup Error");
    html.push_str("<main class=\"setup-shell\"><section class=\"card\"><h1>Setup Error</h1>");
    html.push_str(&format!("<p class=\"notice\">{}</p>", escape_html(message)));
    html.push_str("</section></main></body></html>");
    (status, Html(html)).into_response()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn selected(value: bool) -> &'static str {
    if value { " selected" } else { "" }
}
