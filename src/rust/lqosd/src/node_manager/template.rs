//! Provides an Axum layer that applies templates to static HTML
//! files.

use crate::lts2_sys::shared_types::LtsStatus;
use crate::node_manager::auth::{FIRST_LOAD, get_username};
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::tool_status::is_api_available;
use axum::body::{Body, to_bytes};
use axum::http::header;
use axum::http::{HeaderValue, Request, Response, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use itertools::Itertools;
use lqos_config::load_config;
use lqos_utils::unix_time::unix_now;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;

const VERSION_STRING: &str = include_str!("../../../../VERSION_STRING");

const INSIGHT_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="lnkStats" href="https://insight.libreqos.com/">
        <i class="fa fa-fw fa-centerline fa-line-chart nav-icon"></i> Insight
    </a>
</li>"#;

const INSIGHT_LINK_OFFER_TRIAL: &str = r#"
<li class="nav-item">
    <a class="nav-link text-success" id="lnkStats" href="lts_trial.html">
        <i class="fa fa-line-chart nav-icon"></i> Insight - Free Trial
    </a>
</li>"#;

fn js_tf(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

// Escape HTML special characters for use in attributes
fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// HTML template for API link when available (embedded page)
const API_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="apiLink" href="api_docs.html">
        <i class="fa fa-fw fa-centerline fa-code nav-icon"></i> API Docs
    </a>
    
</li>"#;

// HTML template for API link when unavailable
const API_LINK_INACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="apiLink" href="api.html">
        <i class="fa fa-fw fa-centerline fa-code nav-icon"></i> API Docs
    </a>
</li>"#;

// HTML template for chat link when available (embedded page)
const CHAT_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="chatLink" href="chatbot.html">
        <i class="fa fa-fw fa-centerline fa-comments nav-icon"></i> Ask Libby
    </a>
</li>"#;

static GIT_HASH: &str = env!("GIT_HASH");

pub async fn apply_templates(
    jar: CookieJar,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let apply_template = {
        let path = &req.uri().path().to_string();
        path.ends_with(".html")
    };
    let config = load_config().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            String::from("Cannot load configuration"),
        )
    })?;

    // TODO: Cache this once we're not continually making changes
    let template_text = {
        let path = Path::new(&config.lqos_directory)
            .join("bin")
            .join("static2")
            .join("template.html");
        std::fs::read_to_string(path).expect("Cannot read template html file")
    };

    // Update the displayed username
    let username = get_username(&jar).await;
    let template_text = template_text.replace("%%USERNAME%%", &username);

    let res = next.run(req).await;
    //let mut lts_script = "<script>window.hasLts = false;</script>";
    let mut script_has_lts = false;
    let mut script_has_insight = false;
    let new_version = crate::version_checks::new_version_available();

    if apply_template {
        // Check to see if the box is participating in the Insight Alpha Test
        let mut trial_link;

        // Change the LTS part of the template
        let (lts_status, _) = crate::lts2_sys::get_lts_license_status_async().await;
        trial_link = INSIGHT_LINK_OFFER_TRIAL.to_string();
        match lts_status {
            LtsStatus::Invalid | LtsStatus::NotChecked => {}
            _ => {
                // Link to it
                trial_link = INSIGHT_LINK_ACTIVE.to_string();
                script_has_insight = true;
                script_has_lts = true;
            }
        }

        // Title and node_id
        let mut title = "LibreQoS Node Manager".to_string();
        let mut node_id_js = String::new();
        if let Ok(config) = load_config() {
            title = config.node_name.clone();
            node_id_js = escape_html_attr(&config.node_id);
        }

        // "LTS script" - which is increasingly becoming a misnomer
        let lts_script = format!(
            "<script>window.hasLts = {}; window.hasInsight = {}; window.newVersion = {}; window.nodeId = '{}';</script>",
            js_tf(script_has_lts),
            js_tf(script_has_insight),
            js_tf(new_version),
            node_id_js
        );

        // First Login
        let mut show_modal = "false";
        let mut show_modal_number = "0".to_string();
        if let Ok(now) = unix_now() {
            let week_ago = now - (7 * 24 * 60 * 60);
            let fl = FIRST_LOAD.load(Relaxed);
            if fl != 0 && fl < week_ago {
                let sd = SHAPED_DEVICES.load();
                let num_circuits = sd
                    .devices
                    .iter()
                    .sorted_by(|a, b| a.circuit_hash.cmp(&b.circuit_hash))
                    .dedup()
                    .count();
                if num_circuits > 1_000 && !script_has_insight {
                    show_modal = "true";
                    show_modal_number = num_circuits.to_string();
                }
            }
        }

        let (mut res_parts, res_body) = res.into_parts();
        let bytes = to_bytes(res_body, 1_000_000)
            .await
            .expect("Cannot read template bytes");
        let byte_string = String::from_utf8_lossy(&bytes).to_string();
        let version_string = VERSION_STRING.trim();
        let byte_string = template_text
            .replace("%%BODY%%", &byte_string)
            .replace("%%VERSION%%", version_string)
            .replace("%%TITLE%%", &title)
            .replace("%%LTS_LINK%%", &trial_link)
            .replace("%%%LTS_SCRIPT%%%", &lts_script)
            .replace("%%MODAL%%", &show_modal)
            .replace("%%MODAL_NUM%%", &show_modal_number);
        // Handle API_LINK placeholder (require service + valid Insight)
        let api_link = if is_api_available() && script_has_insight {
            API_LINK_ACTIVE
        } else {
            API_LINK_INACTIVE
        };
        let byte_string = byte_string.replace("%%API_LINK%%", api_link);

        // Handle CHAT_LINK placeholder
        // Libby (chatbot) no longer depends on the local API service being up.
        // Always show the link to the chatbot page; the page will surface availability.
        let chat_link = CHAT_LINK_ACTIVE;
        let byte_string = byte_string.replace("%%CHAT_LINK%%", chat_link);

        // Replace SCHEDULER_STATUS with a simple placeholder for client-side rendering
        // The client JS will fetch status and populate this container.
        let scheduler_placeholder = r##"
<li class="nav-item" id="schedulerStatus">
    <a class="nav-link" href="#" id="schedulerStatusLink">
        <i class="fa fa-fw fa-centerline fa-circle-notch fa-spin"></i> Scheduler
    </a>
</li>
"##;
        let byte_string = byte_string.replace("%%SCHEDULER_STATUS%%", scheduler_placeholder);

        // Placeholder for urgent issues indicator
        let urgent_placeholder = r##"
<li class="nav-item" id="urgentStatus">
    <a class="nav-link text-secondary" href="#" id="urgentStatusLink">
        <i class="fa fa-fw fa-centerline fa-bell-slash"></i> Urgent Issues
        <span id="urgentBadge" class="badge bg-danger d-none">0</span>
    </a>
</li>
"##;
        let byte_string = byte_string.replace("%%URGENT_STATUS%%", urgent_placeholder);

        let byte_string = byte_string.replace("%CACHEBUSTERS%", &format!("?gh={}", GIT_HASH));
        if let Some(length) = res_parts.headers.get_mut("content-length") {
            *length = HeaderValue::from(byte_string.len());
        }
        // Prevent caching of the composed HTML to avoid stale menus/status
        res_parts
            .headers
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        let res = Response::from_parts(res_parts, Body::from(byte_string));
        Ok(res)
    } else {
        Ok(res)
    }
}
