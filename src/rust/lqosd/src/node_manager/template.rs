//! Provides an Axum layer that applies templates to static HTML
//! files.

use crate::node_manager::auth::get_username;
use crate::tool_status::is_api_available;
use axum::body::{Body, to_bytes};
use axum::http::header;
use axum::http::{HeaderValue, Request, Response, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use lqos_config::{RttThresholds, load_config};
use std::path::Path;
use std::time::UNIX_EPOCH;

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

fn cobrand_logo_html(config: &lqos_config::Config) -> String {
    let cobrand_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("cobrand.png");
    if config.display_cobrand && cobrand_path.exists() {
        let cache_buster = std::fs::metadata(&cobrand_path)
            .ok()
            .map(|metadata| {
                let modified_nanos = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0);
                format!("{modified_nanos}-{}", metadata.len())
            })
            .unwrap_or_else(|| "0".to_string());
        format!(
            r#"<img class="lqos_cobrand_logo" src="cobrand.png?v={cache_buster}" alt="" aria-hidden="true" height="48">"#
        )
    } else {
        String::new()
    }
}

fn cobrand_logo_status_html(config: &lqos_config::Config) -> (&'static str, &'static str) {
    let cobrand_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("cobrand.png");
    if config.display_cobrand && cobrand_path.exists() {
        (
            r#"aria-describedby="cobrandLogoStatus""#,
            r#"<span id="cobrandLogoStatus" class="visually-hidden">Custom operator cobrand logo displayed next to the LibreQoS logo in the sidebar.</span>"#,
        )
    } else {
        ("", "")
    }
}

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
    let new_version = crate::version_checks::new_version_available();

    if apply_template {
        // Check to see if the box is participating in the Insight Alpha Test
        let mut trial_link;

        // Change the LTS part of the template
        let capabilities = crate::lts2_sys::current_capabilities();
        trial_link = INSIGHT_LINK_OFFER_TRIAL.to_string();
        if capabilities.can_view_insight_ui {
            trial_link = INSIGHT_LINK_ACTIVE.to_string();
        }

        // Title and node_id
        let title = config.node_name.clone();
        let node_id_js = escape_html_attr(&config.node_id);
        let rtt_thresholds: RttThresholds = config.rtt_thresholds.clone().unwrap_or_default();
        let cobrand_logo = cobrand_logo_html(config.as_ref());
        let (cobrand_logo_describedby, cobrand_logo_status) =
            cobrand_logo_status_html(config.as_ref());

        // "LTS script" - which is increasingly becoming a misnomer
        let lts_script = format!(
            "<script>window.hasLts = {}; window.hasInsight = {}; window.hasSupportTickets = {}; window.hasChatbot = {}; window.hasApiDocs = {}; window.liveControlAvailable = {}; window.licenseStateLabel = {}; window.licenseAuthorityLabel = {}; window.mappedCircuitLimit = {}; window.nodeId = '{}'; window.rttThresholds = {{greenMs: {}, yellowMs: {}, redMs: {}}};</script>",
            js_tf(capabilities.can_view_insight_ui),
            js_tf(capabilities.can_view_insight_ui),
            js_tf(capabilities.can_use_support_tickets),
            js_tf(capabilities.can_use_chatbot),
            js_tf(capabilities.can_use_api_link),
            js_tf(capabilities.control_service_reachable),
            serde_json::to_string(&capabilities.license_state_label)
                .unwrap_or_else(|_| "\"Unknown\"".to_string()),
            serde_json::to_string(&capabilities.authority_label)
                .unwrap_or_else(|_| "\"Unknown\"".to_string()),
            serde_json::to_string(&capabilities.mapped_circuit_limit)
                .unwrap_or_else(|_| "null".to_string()),
            node_id_js,
            rtt_thresholds.green_ms,
            rtt_thresholds.yellow_ms,
            rtt_thresholds.red_ms,
        );

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
            .replace("%%COBRAND_LOGO%%", &cobrand_logo)
            .replace("%%COBRAND_LOGO_DESCRIBEDBY%%", cobrand_logo_describedby)
            .replace("%%COBRAND_LOGO_STATUS%%", cobrand_logo_status);
        // Handle API_LINK placeholder (require service + valid Insight)
        let api_link = if is_api_available() && capabilities.can_use_api_link {
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

        let update_placeholder = if new_version {
            r##"
<li class="nav-item" id="updateStatus">
    <a class="nav-link btn btn-link text-start w-100 p-0 border-0 text-warning" href="https://libreqos.readthedocs.io/en/latest/docs/v2.0/update.html" target="_blank" rel="noopener noreferrer" id="updateStatusLink" aria-label="Open LibreQoS update guide" title="Open LibreQoS update guide">
        <i class="fa fa-fw fa-centerline fa-download"></i> Update Available
    </a>
</li>
"##
        } else {
            ""
        };
        let byte_string = byte_string.replace("%%UPDATE_STATUS%%", update_placeholder);

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

#[cfg(test)]
mod tests {
    use super::{cobrand_logo_html, cobrand_logo_status_html};
    use lqos_config::Config;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const VALID_PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
        b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
        0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', 0x78,
        0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
        0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
    ];

    fn temp_runtime_dir() -> std::path::PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let runtime_dir = std::env::temp_dir().join(format!("lqos-template-test-{unique_suffix}"));
        fs::create_dir_all(runtime_dir.join("bin/static2")).expect("create runtime static dir");
        runtime_dir
    }

    fn test_config(runtime_dir: &std::path::Path, display_cobrand: bool) -> Config {
        let mut config = Config::default();
        config.lqos_directory = runtime_dir.display().to_string();
        config.display_cobrand = display_cobrand;
        config
    }

    #[test]
    fn cobrand_logo_requires_file_and_flag() {
        let runtime_dir = temp_runtime_dir();
        let static_dir = runtime_dir.join("bin/static2");
        let without_file = test_config(&runtime_dir, true);
        let disabled = test_config(&runtime_dir, false);

        assert!(cobrand_logo_html(&without_file).is_empty());
        assert_eq!(cobrand_logo_status_html(&without_file), ("", ""));
        assert!(cobrand_logo_html(&disabled).is_empty());
        assert_eq!(cobrand_logo_status_html(&disabled), ("", ""));

        fs::write(static_dir.join("cobrand.png"), VALID_PNG).expect("write cobrand png");
        let enabled = test_config(&runtime_dir, true);
        let logo_html = cobrand_logo_html(&enabled);
        let (describedby, status_html) = cobrand_logo_status_html(&enabled);

        assert!(logo_html.contains(r#"class="lqos_cobrand_logo""#));
        assert!(logo_html.contains(r#"src="cobrand.png?v="#));
        assert_eq!(describedby, r#"aria-describedby="cobrandLogoStatus""#);
        assert!(status_html.contains("Custom operator cobrand logo displayed"));

        let _ = fs::remove_dir_all(runtime_dir);
    }
}
