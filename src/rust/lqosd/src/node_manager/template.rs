//! Provides an Axum layer that applies templates to static HTML
//! files.

use crate::node_manager::auth::{get_role, get_username};
use crate::node_manager::locale::{
    DEFAULT_UI_LOCALE, dir_for, request_ui_locale, set_content_language,
};
use crate::node_manager::security_headers::apply_node_manager_security_headers;
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

/// Cache-buster query string appended to asset references.
fn cachebuster() -> String {
    format!("?gh={}", GIT_HASH)
}

/// One catalog loader. It carries the cache-buster token, which the shell's
/// final replacement pass fills in.
fn catalog_tag(tag: &str) -> String {
    format!(r#"<script src="i18n/{tag}.js%CACHEBUSTERS%"></script>"#)
}

/// The catalog loaders for `lang`: English always first, because its script
/// publishes the named fallback (`LQOS_I18N_EN`) the runtime reads, then the
/// resolved catalog when it is not English, which publishes the active one.
fn catalog_script_tags(lang: &str) -> String {
    let mut tags = vec![catalog_tag(DEFAULT_UI_LOCALE)];
    if lang != DEFAULT_UI_LOCALE {
        tags.push(catalog_tag(lang));
    }
    tags.join("\n    ")
}

/// Fills the locale placeholders one page carries: the resolved `lang`, the
/// `dir` the caller resolved for it, and the catalog loaders.
///
/// `dir` is a parameter, not a lookup, so the direction path can be rendered
/// from a locale record this build does not ship (see the module's tests).
fn fill_locale_placeholders(html: &str, lang: &str, dir: &str) -> String {
    html.replace("%%LANG%%", lang)
        .replace("%%DIR%%", dir)
        .replace("%%I18N_SCRIPT%%", &catalog_script_tags(lang))
}

/// Applies every substitution the served pages share: the locale placeholders
/// above, then the cache-buster token.
///
/// Both duties live in one pass, because both kinds of `%CACHEBUSTERS%` carrier
/// need it: the catalog loaders the locale pass inserts, and the pages that
/// reference a versioned asset in their own mark-up (`login.html` and
/// `first-run.html` carry the token on the locale list they load). A page that
/// skipped the buster would ship an uncachebusted reference, whichever of the
/// two carriers it holds.
pub(crate) fn apply_page_templates(html: &str, lang: &str) -> String {
    fill_locale_placeholders(html, lang, dir_for(lang)).replace("%CACHEBUSTERS%", &cachebuster())
}

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

/// One sidebar status item structure (the sidebar UI-hygiene card): a single
/// li/a/icon shape so Scheduler/Bridge/Urgent/Update cannot drift.
/// `fa-fw` + the fixed-width `.nav-status-icon` slot keep icons aligned.
///
/// `label` is the English text paired with its catalog key: the text stays
/// inline so the frame still reads correctly without a catalog, and the key is
/// what the browser translates. The key is emitted by Rust only, so
/// `tests::sidebar_status_labels_carry_catalog_keys` is the gate that it exists
/// in both catalogs; `extract.mjs` reads the pair out of the call sites.
fn status_item_html(
    li_id: &str,
    link_id: &str,
    hidden: bool,
    extra_a: &str,
    href: &str,
    icon: &str,
    label: (&str, &str),
    badge: &str,
) -> String {
    let (text, key) = label;
    format!(
        r##"
<li class="nav-item{hidden}" id="{li_id}">
    <a class="nav-link {extra_a}" href="{href}" id="{link_id}">
        <i class="fa fa-fw fa-centerline {icon} nav-status-icon"></i> <span data-i18n="{key}">{text}</span>{badge}
    </a>
</li>
"##,
        hidden = if hidden { " d-none" } else { "" },
    )
}

/// The shell's four sidebar status placeholders, paired with the template token
/// each one fills. The client poll replaces the labels, so these are the
/// server-rendered values; `new_version` is the only conditional one.
fn status_placeholders(new_version: bool) -> [(&'static str, String); 4] {
    [
        // The client JS fetches scheduler status and populates this container.
        (
            "%%SCHEDULER_STATUS%%",
            status_item_html(
                "schedulerStatus",
                "schedulerStatusLink",
                false,
                "",
                "#",
                "fa-circle-notch fa-spin",
                ("Scheduler", "common.nav.scheduler"),
                "",
            ),
        ),
        // E2: bridge Supervisor health placeholder (client-side render; hidden
        // entirely when the DPDK bridge is not configured).
        // L4 §5b: the sidebar Bridge item LINKs to the new Bridge page.
        (
            "%%BRIDGE_STATUS%%",
            status_item_html(
                "bridgeStatus",
                "bridgeStatusLink",
                true,
                "",
                "bridge.html",
                "fa-circle-notch fa-spin",
                ("Bridge", "common.nav.bridge"),
                "",
            ),
        ),
        (
            "%%URGENT_STATUS%%",
            status_item_html(
                "urgentStatus",
                "urgentStatusLink",
                false,
                "text-secondary",
                "#",
                "fa-bell-slash",
                ("Alerts", "common.nav.alerts"),
                r#"<span id="urgentBadge" class="badge bg-danger d-none">0</span>"#,
            ),
        ),
        (
            "%%UPDATE_STATUS%%",
            if new_version {
                status_item_html(
                    "updateStatus",
                    "updateStatusLink",
                    false,
                    "text-warning",
                    // the sidebar UI-hygiene card: the stale v2.0 update page is
                    // replaced by the documentation root until the v3 upgrade doc
                    // is published.
                    "https://libreqos.readthedocs.io/en/latest/",
                    "fa-download",
                    ("Update Available", "common.nav.update_available"),
                    "",
                )
            } else {
                String::new()
            },
        ),
    ]
}

/// The `i18n/` scripts a rendered page loads, in document order: the catalog
/// loaders (`en.js` first, then the served one) followed by `locales.js`, which
/// carries the switcher's locale list rather than messages.
///
/// One helper for both modules that assert on it, and an ordered list rather
/// than a count: a count of two cannot tell `en + es` from `en + en`, and it
/// would have to know the cache-buster spelling.
#[cfg(test)]
pub(crate) fn i18n_scripts(html: &str) -> Vec<&str> {
    const PREFIX: &str = r#"<script src="i18n/"#;
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find(PREFIX) {
        rest = &rest[open + PREFIX.len()..];
        let Some(close) = rest.find('"') else { break };
        let source = &rest[..close];
        scripts.push(source.split_once('?').map_or(source, |(name, _)| name));
        rest = &rest[close..];
    }
    scripts
}

/// The catalog loaders among [`i18n_scripts`]: every served page loads the same
/// catalogs and, beside them, the language selector's generated locale list.
/// Keeping the subtraction here is what stops each caller from re-deriving "a
/// catalog tag" on its own.
#[cfg(test)]
pub(crate) fn catalog_scripts(html: &str) -> Vec<&str> {
    const LOCALE_LIST: &str = "locales.js";
    i18n_scripts(html)
        .into_iter()
        .filter(|name| *name != LOCALE_LIST)
        .collect()
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
    // E3: role surface for UI affordance gating (backend re-checks
    // every write; this only hides buttons from read-only users).
    let role = get_role(&jar).await;
    let template_text = template_text.replace("%%ROLE%%", role);
    // Resolved before the request is handed on: `accept-language` is only
    // readable from the request itself.
    let ui_locale = request_ui_locale(config.ui_language.as_deref(), &jar, req.headers());

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
        let api_service_available = is_api_available();
        let lts_script = format!(
            "<script>window.hasLts = {}; window.hasInsight = {}; window.hasSupportTickets = {}; window.hasChatbot = {}; window.hasApiDocs = {}; window.apiServiceAvailable = {}; window.liveControlAvailable = {}; window.licenseStateLabel = {}; window.licenseAuthorityLabel = {}; window.mappedCircuitLimit = {}; window.nodeId = '{}'; window.rttThresholds = {{greenMs: {}, yellowMs: {}, redMs: {}}};</script>",
            js_tf(capabilities.can_view_insight_ui),
            js_tf(capabilities.can_view_insight_ui),
            js_tf(capabilities.can_use_support_tickets),
            js_tf(capabilities.can_use_chatbot),
            js_tf(capabilities.can_use_api_link),
            js_tf(api_service_available),
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

        // The Site Map's configurable tile base (long_term_stats.lts_url) rides
        // alongside the capability globals; site_map.js reads it (defaults to
        // the historical Insight OSM cache when unset).
        let insight_base = crate::node_manager::local_api::lts::normalize_insight_base_url(
            config.long_term_stats.lts_url.as_deref(),
        );
        let lts_script = format!(
            "<script>window.lqosInsightBaseUrl = {};</script>{}",
            serde_json::to_string(&insight_base).unwrap_or_else(|_| "\"\"".to_string()),
            lts_script
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
        // Handle API_LINK placeholder using service health and current access policy.
        let api_link = if api_service_available && capabilities.can_use_api_link {
            API_LINK_ACTIVE
        } else {
            API_LINK_INACTIVE
        };
        let byte_string = byte_string.replace("%%API_LINK%%", api_link);

        // Handle CHAT_LINK placeholder
        // Libby (chatbot) no longer depends on the local API service being up.
        // Always show the link to the chatbot page; the page will surface availability.
        let chat_link = CHAT_LINK_ACTIVE;
        let mut byte_string = byte_string.replace("%%CHAT_LINK%%", chat_link);

        // The sidebar status items share one shape and one catalog-key emitter.
        for (token, html) in status_placeholders(new_version) {
            byte_string = byte_string.replace(token, &html);
        }

        let byte_string = apply_page_templates(&byte_string, &ui_locale);
        if let Some(length) = res_parts.headers.get_mut("content-length") {
            *length = HeaderValue::from(byte_string.len());
        }
        // Prevent caching of the composed HTML to avoid stale menus/status
        res_parts
            .headers
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        set_content_language(&mut res_parts.headers, &ui_locale)
            .map_err(|status| (status, String::from("Cannot send the UI locale")))?;
        let extra_host = crate::node_manager::security_headers::insight_csp_origin(
            config.long_term_stats.lts_url.as_deref(),
        );
        apply_node_manager_security_headers(&mut res_parts.headers, extra_host.as_deref());
        let res = Response::from_parts(res_parts, Body::from(byte_string));
        Ok(res)
    } else {
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_page_templates, cobrand_logo_html, cobrand_logo_status_html,
        fill_locale_placeholders, i18n_scripts, status_placeholders,
    };
    use crate::node_manager::locale::{dir_for_in, parse_locales};
    use lqos_config::Config;
    use sha2::Digest;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The pages this build serves through the locale placeholders.
    const SHELL_TEMPLATE: &str = include_str!("static2/template.html");
    const LOGIN_TEMPLATE: &str = include_str!("static2/login.html");
    const FIRST_RUN_TEMPLATE: &str = include_str!("static2/first-run.html");

    /// SHA-256 of [`visible_text`] for each English page as it renders today.
    ///
    /// The shell pin held the pre-i18n markup through i18n-02; the frame gained
    /// the Display-menu language switcher in i18n-03, so that value moved with
    /// the intended addition. i18n-06 hoisted that control out of the Display
    /// menu into a sidebar item of its own above it, which moved the word
    /// "Language" from the end of the Display body to just before "Display": the
    /// text set did not change, its order did, so the digest moved with it. The
    /// two standalone pins moved when their footers adopted the shell's
    /// live-year copyright fragments, which changed the visible text from a
    /// frozen "2022-2024" notice. i18n-07 put the language control on both
    /// standalone pages, so those two pins gain that control's one visible word
    /// ("Language", which the catalogs already carried and the sidebar still
    /// shows): the served text grew by the control, not by a new sentence, and
    /// the shell pin held because the sidebar's label text did not change. All
    /// three keep their job as a change detector: any further operator-visible
    /// change to a served page has to be a deliberate edit here.
    ///
    /// Scope: these digests prove "unchanged since i18n-07", not "identical to
    /// develop". All three values were re-anchored inside this branch, and
    /// [`visible_text`] strips tags, so the `data-i18n` attributes this branch
    /// added cannot appear in the comparison. The first-run pin moved once more
    /// when `develop`'s first-run security card added the setup-code row and its
    /// help sentence: that copy is a deliberate operator-visible addition, and
    /// the shell and login pins still hold across the merge.
    const SHELL_VISIBLE_TEXT_SHA256: &str =
        "e171ade1fbb3a3dfe9a6a5825c3664096c2946044ac710d22f55a796fb6402e2";
    const LOGIN_VISIBLE_TEXT_SHA256: &str =
        "81f7d23abffff681a08ce4d7db74cea0a11b613242db18f5d772f1d2c8d8f2cb";
    const FIRST_RUN_VISIBLE_TEXT_SHA256: &str =
        "32a85f656f72f5620e6f53baf47be1ef7c7a95da7d5fb4266eb7196993e8e46d";

    const VALID_PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, b'I',
        b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
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
        Config {
            lqos_directory: runtime_dir.display().to_string(),
            display_cobrand,
            ..Default::default()
        }
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

    /// Text a browser renders: the runs between tags, without comments, script
    /// or style bodies, whitespace collapsed.
    fn visible_text(html: &str) -> String {
        let collapse = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut kept = String::with_capacity(html.len());
        let mut rest = html;
        loop {
            let Some(open) = rest.find('<') else {
                kept.push_str(rest);
                return collapse(&kept);
            };
            kept.push_str(&rest[..open]);
            rest = &rest[open..];
            if let Some(after_open) = rest.strip_prefix("<!--") {
                let Some(end) = after_open.find("-->") else {
                    return collapse(&kept);
                };
                rest = &after_open[end + 3..];
                continue;
            }
            let Some(close) = rest.find('>') else {
                return collapse(&kept);
            };
            let tag = rest[..=close].to_ascii_lowercase();
            rest = &rest[close + 1..];
            let skipped = ["script", "style"]
                .into_iter()
                .find(|name| tag.starts_with(&format!("<{name}")));
            if let Some(skipped) = skipped {
                let end = format!("</{skipped}>");
                let Some(offset) = rest.to_ascii_lowercase().find(&end) else {
                    return collapse(&kept);
                };
                rest = &rest[offset + end.len()..];
            }
        }
    }

    /// The English text of every served page, pinned. Each page carries an
    /// expected phrase so a page the tokenizer failed to parse cannot pass as an
    /// empty string.
    #[test]
    fn english_pages_visible_text_is_unchanged() {
        let pages = [
            (
                "template.html",
                SHELL_TEMPLATE,
                "Logout",
                SHELL_VISIBLE_TEXT_SHA256,
            ),
            (
                "login.html",
                LOGIN_TEMPLATE,
                "Login",
                LOGIN_VISIBLE_TEXT_SHA256,
            ),
            (
                "first-run.html",
                FIRST_RUN_TEMPLATE,
                "First Login",
                FIRST_RUN_VISIBLE_TEXT_SHA256,
            ),
        ];
        for (page, template, expected_phrase, pinned) in pages {
            let english = apply_page_templates(template, "en");
            let text = visible_text(&english);
            assert!(text.contains(expected_phrase) && text.len() > 100, "{page}");
            let digest = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
            assert_eq!(
                digest, pinned,
                "the English text of {page} changed; inspect it and update the pin"
            );
            // Inert in every locale: the substitution only writes attributes and
            // the catalog tags, so no locale changes a visible string.
            let spanish = apply_page_templates(template, "es");
            assert_eq!(
                visible_text(&spanish),
                text,
                "{page} must read the same in every locale"
            );
        }
    }

    /// A locale list this build does not ship: no `rtl` catalog is bundled, so
    /// the records are the only place the right-to-left path can come from.
    const RTL_LOCALES: &str = r#"[{"tag":"en","dir":"ltr"},{"tag":"he","dir":"rtl"}]"#;

    #[test]
    fn direction_comes_from_the_locale_record_when_rendering() {
        let locales = parse_locales(RTL_LOCALES);
        let rtl = dir_for_in(&locales, "he");
        assert_eq!(rtl, "rtl");

        let rendered = fill_locale_placeholders(SHELL_TEMPLATE, "he", rtl);
        assert!(
            rendered.contains(r#"<html lang="he" dir="rtl""#),
            "the shell must render the record's direction: {rendered:.200}"
        );
        // The shipped list carries no `he` record, so a page resolved to a tag
        // this build does not ship still renders left-to-right English.
        assert!(
            apply_page_templates(SHELL_TEMPLATE, "he").contains(r#"dir="ltr""#),
            "an unshipped tag must not invent a direction"
        );
    }

    #[test]
    fn locale_placeholders_are_filled_in() {
        let rendered = apply_page_templates(SHELL_TEMPLATE, "es");

        assert!(rendered.contains(r#"<html lang="es" dir="ltr" data-bs-theme="auto">"#));
        assert_eq!(
            i18n_scripts(&rendered),
            ["en.js", "es.js", "locales.js"],
            "the fallback catalog's script first, then the served catalog, then the switcher's list"
        );
        for token in ["%%LANG%%", "%%DIR%%", "%%I18N_SCRIPT%%"] {
            assert!(!rendered.contains(token), "{token} must not survive");
        }
    }

    #[test]
    fn english_loads_one_catalog_only() {
        let rendered = apply_page_templates(SHELL_TEMPLATE, "en");

        assert_eq!(
            i18n_scripts(&rendered),
            ["en.js", "locales.js"],
            "English is its own fallback, so it asks for one catalog"
        );
    }

    /// Both standalone pages carry the same language control as the shell, so
    /// they have to load the generated locale list the switcher reads: it sits
    /// in the head before the page bundle, and its loader carries a filled
    /// cache-buster rather than a literal template token.
    #[test]
    fn standalone_pages_load_the_language_locale_list() {
        for (page, template) in [
            ("login.html", LOGIN_TEMPLATE),
            ("first-run.html", FIRST_RUN_TEMPLATE),
        ] {
            for lang in ["en", "es"] {
                let rendered = apply_page_templates(template, lang);
                let expected = if lang == "en" {
                    vec!["en.js", "locales.js"]
                } else {
                    vec!["en.js", "es.js", "locales.js"]
                };
                assert_eq!(
                    i18n_scripts(&rendered),
                    expected,
                    "{page} must load its catalogs and then the switcher's list ({lang})"
                );
                assert!(
                    !rendered.contains("%CACHEBUSTERS%"),
                    "{page} must fill the cache-buster its list loader carries"
                );
                let list = rendered
                    .find(r#"<script src="i18n/locales.js"#)
                    .expect("the page must load the generated locale list");
                let bundle_name = page.replace(".html", ".js");
                let bundle = rendered
                    .find(&format!(r#"<script src="{bundle_name}"#))
                    .expect("the page must load its bundle");
                assert!(
                    list < bundle,
                    "{page} must load the locale list before the bundle that builds the control"
                );
            }
        }
    }

    /// The four status labels `template.rs` emits are keyed by Rust, so no
    /// `static2`/`js_build` scan sees them: this test is the gate in both
    /// directions. A typo'd key would silently keep the inline English, and a
    /// key dropped from a catalog would do the same.
    #[test]
    fn sidebar_status_labels_carry_catalog_keys() {
        const STATUS_LABELS: [(&str, &str); 4] = [
            ("Scheduler", "common.nav.scheduler"),
            ("Bridge", "common.nav.bridge"),
            ("Alerts", "common.nav.alerts"),
            ("Update Available", "common.nav.update_available"),
        ];
        const EN_CATALOG: &str = include_str!("static2/i18n/en.json");
        const ES_CATALOG: &str = include_str!("static2/i18n/es.json");
        const PT_BR_CATALOG: &str = include_str!("static2/i18n/pt-BR.json");
        let catalogs = [
            ("en.json", EN_CATALOG),
            ("es.json", ES_CATALOG),
            ("pt-BR.json", PT_BR_CATALOG),
        ]
        .map(|(name, json)| {
            let catalog: serde_json::Value =
                serde_json::from_str(json).expect("the shipped catalog is valid JSON");
            (name, catalog)
        });

        let placeholders = status_placeholders(true);
        assert_eq!(placeholders.len(), 4);
        assert_eq!(placeholders[3].0, "%%UPDATE_STATUS%%");
        let html: String = placeholders.iter().map(|(_, item)| item.as_str()).collect();
        for (label, key) in STATUS_LABELS {
            assert!(
                html.contains(&format!(r#"<span data-i18n="{key}">{label}</span>"#)),
                "the {key} item must render its English label and its catalog key"
            );
            for (name, catalog) in &catalogs {
                assert!(
                    catalog["messages"]
                        .get(key)
                        .is_some_and(|message| message.is_string()),
                    "{name} must define {key}"
                );
            }
        }

        // The update item is the only conditional one: no version, no item.
        assert!(status_placeholders(false)[3].1.is_empty());
    }
}
