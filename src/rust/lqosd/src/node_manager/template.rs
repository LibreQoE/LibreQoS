//! Provides an Axum layer that applies templates to static HTML
//! files.

use crate::lts2_sys::shared_types::LtsStatus;
use crate::node_manager::auth::get_username;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Request, Response, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use lqos_config::load_config;
use std::path::Path;
use crate::tool_status::{is_api_available, is_chatbot_available};

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

const LTS1_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="lnkStats" href="https://stats.libreqos.io/">
        <i class=\"fa fa-line-chart nav-icon\"></i> Statistics
    </a>
</li>
"#;

const LTS1_LINK_OFFER_TRIAL: &str = r#"
<li class="nav-item">
    <a class="nav-link text-success" id="lnkStats" href="%%LTS_TRIAL_LINK%%">
        <i class=\"fa fa-line-chart nav-icon\"></i> Statistics - Free Trial
    </a>
</li>
"#;

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

// HTML template for API link when available
const API_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="apiLink" href="%%API_URL%%" target="_blank" rel="noopener">
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

// HTML template for chat link when available
const CHAT_LINK_ACTIVE: &str = r#"
<li class="nav-item">
    <a class="nav-link" id="chatLink" href="%%CHAT_URL%%" target="_blank" rel="noopener">
        <i class="fa fa-fw fa-centerline fa-comments nav-icon"></i> Ask Libby
    </a>
</li>"#;

// HTML template for chat link when unavailable (rendered as plain text, no hover highlight)
const CHAT_LINK_INACTIVE: &str = r#"
<li class="nav-item">
    <span class="nav-link no-hover" id="chatLink" title="Ask Libby is disabled. Enable it via the API service.">
        <i class="fa fa-fw fa-centerline fa-comments nav-icon"></i> Ask Libby
    </span>
    
</li>"#;

// HTML template for scheduler status when available (without error)
const SCHEDULER_STATUS_ACTIVE: &str = r#"
<li class="nav-item">
    <span class="nav-link text-success">
        <i class="fa fa-fw fa-centerline fa-check-circle"></i> Scheduler
    </span>
</li>"#;

// HTML template for scheduler status when unavailable (without error)
const SCHEDULER_STATUS_INACTIVE: &str = r#"
<li class="nav-item">
    <span class="nav-link text-danger">
        <i class="fa fa-fw fa-centerline fa-times-circle"></i> Scheduler
    </span>
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
    let config = load_config().unwrap();

    // TODO: Cache this once we're not continually making changes
    let template_text = {
        let path = Path::new(&config.lqos_directory)
            .join("bin")
            .join("static2")
            .join("template.html");
        std::fs::read_to_string(path).unwrap()
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

        // Title
        let mut title = "LibreQoS Node Manager".to_string();
        if let Ok(config) = load_config() {
            title = config.node_name.clone();
        }

        // "LTS script" - which is increasingly becoming a misnomer
        let lts_script = format!(
            "<script>window.hasLts = {}; window.hasInsight = {}; window.newVersion = {};</script>",
            js_tf(script_has_lts),
            js_tf(script_has_insight),
            js_tf(new_version)
        );

        let (mut res_parts, res_body) = res.into_parts();
        let bytes = to_bytes(res_body, 1_000_000).await.unwrap();
        let byte_string = String::from_utf8_lossy(&bytes).to_string();
        let byte_string = template_text
            .replace("%%BODY%%", &byte_string)
            .replace("%%VERSION%%", VERSION_STRING)
            .replace("%%TITLE%%", &title)
            .replace("%%LTS_LINK%%", &trial_link)
            .replace("%%%LTS_SCRIPT%%%", &lts_script);
        // Handle API_LINK placeholder
        let api_link = if is_api_available() {
            API_LINK_ACTIVE
        } else {
            API_LINK_INACTIVE
        };
        let byte_string = byte_string.replace("%%API_LINK%%", api_link);

        // Handle CHAT_LINK placeholder (visible even when unavailable)
        let chat_link = if is_chatbot_available() {
            CHAT_LINK_ACTIVE
        } else {
            CHAT_LINK_INACTIVE
        };
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

        let byte_string = byte_string
            .replace("%CACHEBUSTERS%", &format!("?gh={}", GIT_HASH));
        if let Some(length) = res_parts.headers.get_mut("content-length") {
            *length = HeaderValue::from(byte_string.len());
        }
        let res = Response::from_parts(res_parts, Body::from(byte_string));
        Ok(res)
    } else {
        Ok(res)
    }
}
