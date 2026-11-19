use crate::node_manager::auth::auth_layer;
use crate::node_manager::template::apply_templates;
use anyhow::{Result, bail};
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::convert::Infallible;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::Service;
use tower_http::services::{ServeDir, ServeFile};

/// Every templated/authenticated HTML shell. Security card 94b00da77 §3.7
/// (invariant 8): this list is the ONLY way a page is served; the fallback is
/// assets-only and must reject `*.html`, so an unlisted shell is a build/test
/// failure rather than an unauthenticated serve. The completeness test asserts
/// this list + the separately-routed shells equals `static2/*.html`.
pub(super) const HTML_PAGES: &[&str] = &[
    "index.html",
    "shaped_devices.html",
    "tree.html",
    "topology_manager.html",
    "topology_probes.html",
    "bridge.html",
    "cpu_workers.html",
    "help.html",
    "unknown_ips.html",
    "circuit.html",
    "ethernet_caps.html",
    "site_map.html",
    "flow_map.html",
    "all_tree_sankey.html",
    "asn_analysis.html",
    "asn_explorer.html",
    "api_docs.html",
    "chatbot.html",
    "lts_trial.html",
    "lts_trial_setup.html",
    "lts_trial_success.html",
    "lts_trial_fail.html",
    "executive_worst_sites.html",
    "executive_oversubscribed_sites.html",
    "executive_sites_due_upgrade.html",
    "executive_circuits_due_upgrade.html",
    "executive_top_asns.html",
    "executive_heatmap_rtt.html",
    "executive_heatmap_retransmit.html",
    "executive_heatmap_download.html",
    "executive_heatmap_upload.html",
    "config_general.html",
    "config_ssl.html",
    "config_rtt.html",
    "config_tuning.html",
    "config_queues.html",
    "config_dataplane.html",
    "config_lts.html",
    "config_iprange.html",
    "config_flows.html",
    "config_dynamic_circuits.html",
    "config_radius_accounting.html",
    "config_integration.html",
    "config_splynx.html",
    "config_netzur.html",
    "config_visp.html",
    "config_uisp.html",
    "config_powercode.html",
    "config_sonar.html",
    "setup_runtime.html",
    "config_interface.html",
    "config_network.html",
    "config_devices.html",
    "config_users.html",
    "config_wispgate.html",
    "config_stormguard.html",
    "stormguard_debug.html",
    "api.html",
];

/// The asset fallback: static files, but NEVER an HTML shell. A `*.html`
/// request that did not match an explicit route is a 404 (a build/test failure
/// surfaces here as a missing page, not an unauthenticated serve).
///
/// The future is boxed on purpose: the debug-build `ServeDir` future is large
/// enough to overflow the 2 MiB test-thread stack on a miss, and a nested
/// router makes it larger still. Boxing keeps the stack frame small.
pub(super) fn asset_fallback(static_root: &Path) -> AssetFallback {
    AssetFallback {
        // Never synthesize `<dir>/index.html` for a directory request: the
        // shell must only be reachable through its explicit route.
        serve_dir: ServeDir::new(static_root).append_index_html_on_directories(false),
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decode `%XX` escapes exactly once — the same transform tower-http applies
/// before it resolves a file. A malformed escape returns `None` (refuse).
fn percent_decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = hex_nibble(*bytes.get(index + 1)?)?;
            let lo = hex_nibble(*bytes.get(index + 2)?)?;
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// True for a path whose final segment is an HTML file (any ASCII case).
fn is_html_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .map(|segment| segment.to_ascii_lowercase().ends_with(".html"))
        .unwrap_or(false)
}

/// Concrete fallback service. The future is an explicit boxed `Send` future so
/// the very large debug-build `ServeDir` future lives on the heap: a miss must
/// not overflow the 2 MiB test-thread stack.
#[derive(Clone)]
pub(super) struct AssetFallback {
    serve_dir: ServeDir,
}

impl Service<Request> for AssetFallback {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::<Request>::poll_ready(&mut self.serve_dir, cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        // Decode BEFORE the check: tower-http percent-decodes the path, so a
        // raw-URI suffix test would serve `/x%2Ehtml`. A malformed escape is
        // refused. `append_index_html_on_directories(false)` covers `<dir>/`.
        let decoded = percent_decode_path(req.uri().path());
        if decoded.as_deref().map(is_html_path).unwrap_or(true) {
            return Box::pin(async { Ok::<_, Infallible>(StatusCode::NOT_FOUND.into_response()) });
        }
        let mut serve_dir = self.serve_dir.clone();
        Box::pin(async move {
            let response = Service::<Request>::call(&mut serve_dir, req).await?;
            Ok::<_, Infallible>(response.into_response())
        })
    }
}

/// Returns the static file service that serves vendored assets from
/// `<static_root>/vendor`, for mounting with `Router::nest_service`.
///
/// Fails when the vendor directory is missing, so a deployment with an
/// incomplete `static2` tree is reported at startup rather than served empty.
pub(super) fn vendor_service(static_root: &Path) -> Result<ServeDir> {
    let vendor_path = static_root.join("vendor");

    if !vendor_path.exists() {
        bail!(
            "Vendor path not found for webserver ({})",
            vendor_path.display()
        );
    }

    Ok(ServeDir::new(vendor_path))
}

/// Builds the router that serves the templated HTML pages from `static_root`.
///
/// Authentication and templating are applied as route layers, so only the
/// pages listed here receive them. Fails when a listed page is missing.
pub(super) fn static_routes(static_root: &Path) -> Result<Router> {
    // Iterate through pages and construct the router
    let mut router = Router::new();
    for page in HTML_PAGES.iter() {
        let path = static_root.join(page);

        if !path.exists() {
            bail!("Missing webpage: {page}");
        }

        router = router.route_service(&format!("/{page}"), ServeFile::new(path));
    }

    // Backwards compatible alias for historical misspelling.
    // This route is included in the templated/authenticated router so bookmarks keep working.
    let splynx_path = static_root.join("config_splynx.html");
    router = router.route_service("/config_spylnx.html", ServeFile::new(splynx_path));

    router = router
        .route_layer(axum::middleware::from_fn(auth_layer))
        .route_layer(axum::middleware::from_fn(apply_templates));

    Ok(router)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::fs;
    use tower::ServiceExt;

    /// Security card 94b00da77 §3.7 (invariant 8): every `static2/*.html` shell
    /// is either in the explicit templated list or has its own explicit route.
    /// An unlisted page is a build/test failure, never an unauthenticated serve.
    #[test]
    fn every_static_html_shell_is_explicitly_routed() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node_manager/static2");
        let mut on_disk: Vec<String> = fs::read_dir(&dir)
            .expect("static2 tree")
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("html") {
                    Some(path.file_name()?.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect();
        on_disk.sort();

        let mut routed: Vec<String> = HTML_PAGES.iter().map(|page| (*page).to_string()).collect();
        // Routed directly in run.rs rather than through the templated list.
        for page in ["login.html", "first-run.html", "template.html"] {
            routed.push(page.to_string());
        }
        routed.sort();

        assert_eq!(
            on_disk, routed,
            "a static2/*.html shell must be explicitly routed (templated list or run.rs)"
        );
    }

    /// The fallback serves assets only: an unlisted `*.html` is a 404, never an
    /// unauthenticated file serve.
    #[tokio::test]
    async fn the_asset_fallback_refuses_html_but_serves_assets() {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-asset-fallback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(dir.join("sub")).expect("create subdir");
        fs::write(dir.join("app.js"), b"console.log(1);").expect("write asset");
        fs::write(dir.join("orphan.html"), b"<html>orphan</html>").expect("write orphan page");
        fs::write(
            dir.join("sub/index.html"),
            b"<html>secret-index</html>",
        )
        .expect("write unlisted directory index");

        let app = asset_fallback(&dir);
        for path in [
            "/orphan.html",
            "/orphan%2Ehtml",
            "/orphan%2ehtml",
            "/orphan%2EHTML",
            "/sub/",
            "/sub",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .expect("fallback answers");
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body");
            let body = String::from_utf8_lossy(&body);
            assert_ne!(
                status,
                StatusCode::OK,
                "{path} must not be served by the fallback"
            );
            assert!(
                !body.contains("secret-index") && !body.contains("orphan"),
                "{path} leaked an unlisted HTML shell: {body}"
            );
        }

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("fallback answers");
        assert_eq!(response.status(), StatusCode::OK);

        let _ = fs::remove_dir_all(&dir);
    }
}
