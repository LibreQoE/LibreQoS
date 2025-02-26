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
        let has_insight = config.long_term_stats.use_insight.unwrap_or(false);
        let mut trial_link;

        if has_insight {
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
        } else {
            if config.long_term_stats.gather_stats && config.long_term_stats.license_key.is_some() {
                // LTS is enabled
                trial_link = LTS1_LINK_ACTIVE.to_string();
                script_has_lts = true;
                script_has_insight = false;
            } else {
                trial_link = LTS1_LINK_OFFER_TRIAL.replace(
                    "%%LTS_TRIAL_LINK%%",
                    &format!("https://stats.libreqos.io/trial1/{}", config.node_id),
                );
                script_has_insight = false;
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
        if let Some(length) = res_parts.headers.get_mut("content-length") {
            *length = HeaderValue::from(byte_string.len());
        }
        let res = Response::from_parts(res_parts, Body::from(byte_string));
        Ok(res)
    } else {
        Ok(res)
    }
}
