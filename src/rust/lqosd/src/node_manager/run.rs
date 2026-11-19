use crate::lts2_sys::control_channel::ControlChannelCommand;
use crate::node_manager::local_api::api_key_auth;
use crate::node_manager::local_api::local_api;
use crate::node_manager::security_headers::apply_api_docs_security_headers;
use crate::node_manager::shaper_queries_actor::{ShaperQueryCommand, shaper_queries_actor};
use crate::node_manager::{
    auth,
    static_pages::{asset_fallback, static_routes, vendor_service},
    ws::websocket_router,
};
use crate::system_stats::SystemStats;
use anyhow::{Result, bail};
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use lqos_bus::BusRequest;
use lqos_config::load_config;
use lqos_probe::ProbeClient;
use std::path::Path;
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use tower_http::services::ServeFile;
use tracing::info;

/// The WebUI listen address when `webserver_listen` is absent or blank.
///
/// beta20.8 (driver ruling 2026-11-14): this is the **direct plaintext WebUI
/// listener**, restored to all interfaces so an existing beta9 install with no
/// `webserver_listen` keeps working after upgrade (the pre-Nov-11 upgrade
/// behaviour). The exposure is driver-accepted ("installers know not to use
/// publicly routed IPs"); the auth/setup gates are unchanged.
///
/// IPv6 narrowing (recorded): the pre-Nov-11 default was the dual-stack
/// `:::9123`; `0.0.0.0:9123` is **IPv4-only**. An IPv6-only host must set
/// `webserver_listen = "[::]:9123"` (or `[::1]:9123` for loopback). This is
/// NOT byte-identical to the old default.
///
/// The TLS-terminating Caddy proxy is separate and always loopback
/// (`lqos_setup::ssl::{RUNTIME_DIRECT_LISTEN, RUNTIME_SECURE_LISTEN,
/// WEB_UPSTREAM}` = `127.0.0.1:9123`), so an SSL-enabled install does not
/// expose this listener.
pub(crate) const DEFAULT_WEBSERVER_LISTEN: &str = "0.0.0.0:9123";

/// Resolve the listener: the operator's configured bind, or the default.
/// An empty/blank value is treated as unset. An explicit value wins verbatim
/// (loopback `127.0.0.1:9123` and IPv6 `[::]:9123` included).
fn resolved_webserver_listen(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_WEBSERVER_LISTEN.to_string())
}

/// True when `webserver_listen` is absent/blank (the default bind applies).
fn webserver_listen_is_default(configured: Option<&str>) -> bool {
    configured
        .map(str::trim)
        .map(|value| value.is_empty())
        .unwrap_or(true)
}

/// Launches the Axum webserver to take over node manager duties.
/// This is designed to be run as an independent Tokio future,
/// with tokio::spawn unless you want it to block execution.
pub async fn spawn_webserver(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
    probe_client: ProbeClient,
) -> Result<()> {
    // Check that static content is available and set up the path.
    // 3b (rev 857): the webserver serves the RUNTIME directory
    // `<lqos_directory>/bin/static2` — NOT the source `node_manager/static2`.
    // A code change to the JS/HTML must be built (`js_build/esbuild.sh`) and
    // COPIED into the served directory at the binary swap, or the old bundles
    // are served regardless of the tree.
    let config = load_config()?;
    let static_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2");

    if !static_path.exists() {
        bail!("Static path not found for webserver (bin/static2/");
    }

    // Security card 1ad21622a §3.1: mint the one-time setup code at
    // startup/reset ONLY while unconfigured (never on the request path).
    super::auth::ensure_first_run_setup_code();

    // Listen for net connections. Unset => loopback (the bind is the network
    // control; never a wildcard default).
    let listen_address = resolved_webserver_listen(config.webserver_listen.as_deref());
    let listener = TcpListener::bind(&listen_address).await?;

    // Setup shaper queries
    let shaper_tx = shaper_queries_actor(control_tx.clone()).await;

    // The embedded v2 API (/api/v3): same listener, key-only auth. Its state is
    // the daemon's own handles — the mount starts no sampler, watcher or
    // topology detector of its own.
    let api_state = api_state();

    // Construct the router from parts
    let router = build_router(
        &static_path,
        bus_tx,
        system_usage_tx,
        control_tx,
        probe_client,
        shaper_tx,
        api_state,
    )?;

    // Name the bound address AND its source: an operator upgrading from beta9
    // must be able to see the restored wildcard default in the startup log.
    let listen_source = if webserver_listen_is_default(config.webserver_listen.as_deref()) {
        "default: webserver_listen absent/blank"
    } else {
        "configured via webserver_listen"
    };
    info!("Webserver listening on: [{listen_address}] ({listen_source})");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// The mount's state, built from the daemon's own inputs.
///
/// The API crate keeps router construction side-effect free and owns no
/// process-wide input. The daemon already has all three: the shaping-CPU
/// topology (detected once in `main`), the 1 Hz system-stats sampler (started
/// in `main`), and the shaped-devices catalog the circuit directory comes from.
/// This is the single place they are handed over, so the wiring is testable as
/// a unit instead of three lines a reader has to trust.
fn api_state() -> std::sync::Arc<lqos_api::api::ApiState> {
    lqos_api::api::ApiState::new(
        crate::node_manager::local_api::cpu_workers::topology_cell(),
        system_usage_snapshot,
        circuit_name_directory,
    )
}

/// The daemon's 1 Hz CPU/RAM sampler, in the mount's snapshot shape.
fn system_usage_snapshot() -> lqos_api::api::SystemUsageSnapshot {
    use std::sync::atomic::Ordering;

    let cpu = crate::system_stats::cpu_usage_snapshot();
    lqos_api::api::SystemUsageSnapshot {
        cpu_usage: cpu.cpu_usage,
        cpu_ids: cpu.cpu_ids,
        num_cpus: cpu.num_cpus,
        ram_used: crate::system_stats::RAM_USED.load(Ordering::Relaxed),
        total_ram: crate::system_stats::TOTAL_RAM.load(Ordering::Relaxed),
    }
}

/// `circuit_id -> circuit_name`, from the same catalog the daemon shapes from.
fn circuit_name_directory() -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;

    let catalog = lqos_network_devices::shaped_devices_catalog();
    let mut names: HashMap<String, String> = HashMap::new();
    for device in catalog.iter_devices() {
        names
            .entry(device.circuit_id.clone())
            .or_insert_with(|| device.circuit_name.clone());
    }
    names
}

/// Builds the complete node manager router from the static asset root and the
/// daemon-side channels the handlers use.
///
/// Fails when the static asset tree is incomplete. Panics while registering a
/// route whose path syntax is invalid, which is why the router is constructed
/// as one unit before the listener starts serving.
///
/// `/api/v3` is the v2 `lqos_api` surface mounted on this same router, outside
/// the node manager's cookie `auth_layer`: it is key-only (`x-bearer`) and its
/// unknown paths answer JSON, never the static fallback's HTML. The API's own
/// documentation is merged beside that mount, so it answers without a key.
fn build_router(
    static_path: &Path,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
    probe_client: ProbeClient,
    shaper_tx: Sender<ShaperQueryCommand>,
    api_state: std::sync::Arc<lqos_api::api::ApiState>,
) -> Result<Router> {
    Ok(Router::new()
        .route("/", get(redirect_to_index))
        .route("/login.html", get(auth::login_page))
        .route("/first-run.html", get(auth::first_run_page))
        .route("/doLogin", post(auth::try_login))
        .route("/firstLogin", post(auth::first_user))
        .route("/health", get(health_check))
        .route("/template.html", get(not_found))
        .route("/configuration.html", get(redirect_configuration_page))
        // Backwards compatible aliases for historical misspellings.
        .route_service(
            "/config_spylnx.js",
            ServeFile::new(static_path.join("config_splynx.js")),
        )
        .route_service(
            "/config_spylnx.js.map",
            ServeFile::new(static_path.join("config_splynx.js.map")),
        )
        .nest(
            "/websocket/",
            websocket_router(
                bus_tx.clone(),
                system_usage_tx.clone(),
                control_tx.clone(),
                probe_client.clone(),
                shaper_tx.clone(),
            ),
        )
        .nest_service("/vendor", vendor_service(static_path)?) // Serve /vendor as purely static
        .merge(static_routes(static_path)?)
        .nest("/local-api", local_api(shaper_tx))
        .nest(
            lqos_api::web::API_MOUNT_PREFIX,
            // `.layer`, not `.route_layer`: the mount's own JSON-404 fallback and
            // its method-not-allowed responses must sit INSIDE the auth boundary,
            // so an unauthenticated caller cannot enumerate registered paths.
            lqos_api::api_routes(api_state)
                .layer(axum::middleware::from_fn(api_key_auth::require_api_key)),
        )
        // The API's documentation is public. Merged BESIDE the nest, never
        // nested inside it, so `/docs` and the spec answer without a key while
        // every data route keeps the boundary above.
        .merge(
            lqos_api::web::docs_router()
                .layer(axum::middleware::from_fn(apply_api_docs_security_headers)),
        )
        // Assets only: `*.html` never falls through to an unauthenticated
        // ServeDir (card 94b00da77 §3.7, invariant 8).
        .fallback_service(asset_fallback(static_path)))
}

/// Provides a redirect service that sends visitors
/// to the index.html page.
async fn redirect_to_index() -> Redirect {
    Redirect::permanent("/index.html")
}

async fn redirect_configuration_page() -> Redirect {
    Redirect::permanent("/config_devices.html")
}

/// Provides a simple OK status
async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

async fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// beta20.8 upgrade regression (driver ruling 2026-11-14): an absent or
    /// blank `webserver_listen` binds all IPv4 interfaces again, so a beta9
    /// install with no `webserver_listen` stays reachable after upgrade.
    /// An explicit value wins verbatim, including loopback and IPv6.
    #[test]
    fn unset_webserver_listen_defaults_to_all_ipv4_interfaces() {
        assert_eq!(resolved_webserver_listen(None), "0.0.0.0:9123");
        assert_eq!(resolved_webserver_listen(Some("")), "0.0.0.0:9123");
        assert_eq!(resolved_webserver_listen(Some("   ")), "0.0.0.0:9123");
        // Explicit values win verbatim.
        assert_eq!(
            resolved_webserver_listen(Some("203.0.113.5:9123")),
            "203.0.113.5:9123"
        );
        assert_eq!(
            resolved_webserver_listen(Some("127.0.0.1:9123")),
            "127.0.0.1:9123"
        );
        assert_eq!(resolved_webserver_listen(Some("[::]:9123")), "[::]:9123");
        assert_eq!(resolved_webserver_listen(Some("[::1]:9123")), "[::1]:9123");
        assert_eq!(DEFAULT_WEBSERVER_LISTEN, "0.0.0.0:9123");
        // Source detection for the startup log.
        assert!(webserver_listen_is_default(None));
        assert!(webserver_listen_is_default(Some("")));
        assert!(webserver_listen_is_default(Some("  ")));
        assert!(!webserver_listen_is_default(Some("127.0.0.1:9123")));
    }

    /// The resolved default really is a wildcard bind: bind its HOST on an
    /// ephemeral port (9123 may be in use on a bench box) and assert the
    /// kernel reports an unspecified local address.
    #[test]
    fn resolved_default_binds_the_wildcard_address() {
        let resolved = resolved_webserver_listen(None);
        let host = resolved.rsplit_once(':').expect("host:port").0;
        let listener =
            std::net::TcpListener::bind(format!("{host}:0")).expect("bind the default host");
        assert!(
            listener.local_addr().expect("local addr").ip().is_unspecified(),
            "the default host must bind all IPv4 interfaces"
        );
    }

    /// The help text the operator reads must name the restored default (the
    /// stale side was the code, not the UI).
    #[test]
    fn ui_help_text_names_the_restored_default() {
        for (name, doc) in [
            ("config_general.html", include_str!("static2/config_general.html")),
            ("i18n/en.json", include_str!("static2/i18n/en.json")),
            ("i18n/es.json", include_str!("static2/i18n/es.json")),
        ] {
            assert!(
                doc.contains("0.0.0.0:9123"),
                "{name} must name the restored default 0.0.0.0:9123"
            );
        }
    }

    #[test]
    fn node_manager_router_sources_do_not_enable_cors_layer() {
        let sources = [
            include_str!("run.rs"),
            include_str!("local_api.rs"),
            include_str!("ws.rs"),
        ];
        for source in sources {
            assert!(!source.contains(concat!("Cors", "Layer")));
            assert!(!source.contains(concat!("tower_http::", "cors")));
        }
    }

    /// The daemon must be able to build the complete node manager router.
    ///
    /// axum panics while registering a route whose path syntax is invalid, so a
    /// successful build is the guard for every route string in the tree:
    /// `/health`, `/websocket/ws`, `/local-api/pcapDump/{id}`, the templated
    /// pages such as `/index.html`, and the mounted `/api/v3` surface. The
    /// in-tree `static2` tree is the fixture, so the check does not depend on an
    /// installed LibreQoS.
    #[tokio::test]
    async fn node_manager_router_builds_with_the_full_route_tree() {
        let _router = test_router();
    }

    /// The sentinel CPU list: 4242% is not a possible measurement, so an answer
    /// carrying it can only have come from the injected handle.
    const SENTINEL_CPU_USAGE: [u32; 2] = [4242, 4243];

    fn sentinel_system_usage() -> lqos_api::api::SystemUsageSnapshot {
        lqos_api::api::SystemUsageSnapshot {
            cpu_usage: SENTINEL_CPU_USAGE.to_vec(),
            cpu_ids: vec![0, 1],
            num_cpus: SENTINEL_CPU_USAGE.len(),
            ram_used: 424_242,
            total_ram: 999_999,
        }
    }

    /// Every system reading the mount serves comes from the injected handle.
    ///
    /// The sentinel values are impossible measurements, so this FAILS for any
    /// mount that samples the machine itself: it would answer the host's real
    /// CPU percentages, not 4242%. That is the provenance claim the previous
    /// version of this test could not make — comparing a live host sample with
    /// another live host sample agrees no matter which code produced it.
    #[tokio::test]
    async fn api_v3_serves_the_injected_daemon_snapshot() {
        let config = ApiKeyConfig::new("api-v3-sentinel");
        let router = test_router_with(lqos_api::api::ApiState::new(
            crate::node_manager::local_api::cpu_workers::topology_cell(),
            sentinel_system_usage,
            circuit_name_directory,
        ));

        let (status, json) =
            json_get(router.clone(), "/api/v3/cpu_usage", Some(config.api_key())).await;
        assert_eq!(status, StatusCode::OK, "cpu_usage: {json}");
        assert_eq!(
            json,
            serde_json::json!(SENTINEL_CPU_USAGE),
            "the mount must serve the injected CPU sample"
        );

        let (status, json) =
            json_get(router.clone(), "/api/v3/ram_usage", Some(config.api_key())).await;
        assert_eq!(status, StatusCode::OK, "ram_usage: {json}");
        assert_eq!(
            json["used"], 424_242,
            "the mount must serve the injected RAM sample"
        );
        assert_eq!(json["total"], 999_999);

        let (status, json) = json_get(
            router.clone(),
            "/api/v3/system_stats_health",
            Some(config.api_key()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "system_stats_health: {json}");
        assert_eq!(json["num_cpus"], 2);
        assert_eq!(json["ram_used"], 424_242);

        // The CPU & Workers section reads the same injected handle.
        let (status, json) = json_get(router, "/api/v3/cpu_workers", Some(config.api_key())).await;
        assert_eq!(status, StatusCode::OK, "cpu_workers: {json}");
        let percents: Vec<u32> = json["cpu_usage"]["per_cpu"]
            .as_array()
            .expect("cpu_workers must carry per-CPU rows")
            .iter()
            .map(|row| row["percent"].as_u64().expect("percent") as u32)
            .collect();
        assert_eq!(
            percents,
            SENTINEL_CPU_USAGE.to_vec(),
            "cpu_workers must read the injected sample, got {}",
            json["cpu_usage"]
        );
    }

    /// `/cpu_workers` topology IS the daemon's cell, not merely "some detection".
    ///
    /// The assertion is IDENTITY: the JSON the mount serves must equal the section
    /// stored in `topology_cell()`. That is what a returned-to-sender detection
    /// cannot pass — an API-side `OnceLock` of its own (the wiring this unit
    /// removed) would leave the daemon's cell unread, so the mount would answer
    /// `topology_pending` while the cell holds a detected section, and the two
    /// would differ. `init_topology` is idempotent (`OnceLock::set` keeps the
    /// first value), so re-running this test is harmless; no other test that reads
    /// the cell runs concurrently with a different expectation, so `#[serial]` is
    /// not needed (and `serial_test` is not a dependency of this crate).
    #[tokio::test]
    async fn api_v3_reads_the_daemons_topology_snapshot() {
        let config = ApiKeyConfig::new("api-v3-topology");
        let cell = crate::node_manager::local_api::cpu_workers::topology_cell();

        // The daemon's startup detection (the `main` call).
        crate::node_manager::local_api::cpu_workers::init_topology(&lqos_config::Config::default());

        let detected = cell
            .get()
            .cloned()
            .flatten()
            .expect("init_topology must fill the daemon's cell");
        assert!(detected.available, "the detection itself must be available");

        let (status, json) =
            json_get(test_router(), "/api/v3/cpu_workers", Some(config.api_key())).await;
        assert_eq!(status, StatusCode::OK, "cpu_workers: {json}");
        assert_eq!(
            json["cpu_topology"],
            serde_json::to_value(&detected).expect("the section serialises"),
            "the mount must serve the daemon's own cell"
        );
        assert!(
            json["cpu_topology"]["shaping"].is_array(),
            "the detected shaping-CPU list must come through: {}",
            json["cpu_topology"]
        );
    }

    /// An empty cell is reported as `topology_pending`, never as a guess.
    ///
    /// A separate cell stands in for "detection has not run": the process-global
    /// one is set by the test above and `OnceLock` cannot be reset, so the
    /// baseline is proven on a fresh cell instead of by test ordering.
    #[tokio::test]
    async fn an_uninitialised_topology_cell_reports_topology_pending() {
        static EMPTY_CELL: std::sync::OnceLock<Option<lqos_api::api::CpuTopologySection>> =
            std::sync::OnceLock::new();
        let config = ApiKeyConfig::new("api-v3-topology-pending");

        let section = lqos_api::api::ApiState::new(
            &EMPTY_CELL,
            sentinel_system_usage,
            circuit_name_directory,
        )
        .cpu_topology();
        assert!(!section.available);
        assert_eq!(section.reason.as_deref(), Some("topology_pending"));

        // The mounted surface says the same thing for that state.
        let router = test_router_with(lqos_api::api::ApiState::new(
            &EMPTY_CELL,
            sentinel_system_usage,
            circuit_name_directory,
        ));
        let (status, json) = json_get(router, "/api/v3/cpu_workers", Some(config.api_key())).await;
        assert_eq!(status, StatusCode::OK, "cpu_workers: {json}");
        assert_eq!(json["cpu_topology"]["available"], false);
        assert_eq!(json["cpu_topology"]["reason"], "topology_pending");
    }

    /// The daemon adapter forwards the live sampler, not a constant.
    ///
    /// The processor count is the machine's, so it cannot be faked; waiting for
    /// the first tick and matching it pins the wiring end to end.
    #[tokio::test]
    async fn api_v3_system_sample_is_the_daemons_live_sampler() {
        let _usage_tx = crate::system_stats::start_system_stats()
            .expect("the daemon's system-stats sampler must start");
        let daemon = wait_for_sampler().await;

        let reported = api_state().system_stats();
        assert_eq!(
            reported.num_cpus, daemon.num_cpus,
            "the mount's handle must read the daemon's sampler"
        );
        assert_eq!(reported.cpu_usage, daemon.cpu_usage);
        assert_eq!(reported.cpu_ids, daemon.cpu_ids);
    }

    /// Waits for the daemon's sampler to produce its first tick, then returns the
    /// snapshot the mount should be answering from.
    async fn wait_for_sampler() -> crate::system_stats::CpuUsageSnapshot {
        for _ in 0..300 {
            let snapshot = crate::system_stats::cpu_usage_snapshot();
            if snapshot.num_cpus > 0 {
                return snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("the daemon's system-stats sampler never produced a tick");
    }

    /// A `GET` that parses the JSON body, for the endpoints whose answer shape
    /// is part of the contract.
    async fn json_get(
        router: Router,
        path: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let (status, _, body) = request_with_headers(router, path, bearer).await;
        let json = serde_json::from_str(&body).unwrap_or_else(|error| {
            panic!("{path} must answer JSON ({error}): {body}");
        });
        (status, json)
    }

    /// The local-api routes dispatch, not just register.
    ///
    /// `/local-api/pcapDump/{id}` survived the axum 0.8 path-syntax change
    /// (`:id` → `{id}`). A session cannot be minted from here
    /// (`build_session_cookie`/`session_key` are private to `auth.rs`), so the
    /// proof is split in two:
    /// * through the REAL node-manager router, `/local-api/pcapDump/3` reaches
    ///   the auth layer and gets its redirect. That answer is decisive: the
    ///   local-api router applies `auth_layer` with `route_layer`, which does not
    ///   wrap the nested fallback, so the static fallback can only serve a file
    ///   or answer 404 — it cannot redirect, and only a matched route can;
    /// * on an axum router carrying the same handler under the same `{id}` path
    ///   form, a non-numeric id is rejected by `Path<usize>` decoding (400) and
    ///   a numeric id reaches the handler (404: no capture for that id). That is
    ///   the decoding a session would have let the real router exercise.
    ///
    /// The unrouted-sibling contrast (`/local-api/no-such-route/3` → static 404)
    /// is deliberately not asserted here: the node-manager fallback is a
    /// `ServeDir`, whose debug-build future overflows the 2 MiB test-thread stack
    /// on a miss (the same request answers 404 under `RUST_MIN_STACK=32M`). That
    /// is a harness limitation, not routing behaviour, and the `route_layer`
    /// argument above already excludes the fallback as the source of the
    /// redirect.
    #[tokio::test]
    async fn local_api_pcap_dump_route_dispatches_after_the_axum_0_8_path_change() {
        let router = test_router();

        let (status, headers, _) =
            request_with_headers(router, "/local-api/pcapDump/3", None).await;
        assert!(
            status.is_redirection(),
            "the auth layer must intercept the pcap route, got {status}"
        );
        let location = headers
            .get("location")
            .and_then(|value| value.to_str().ok());
        assert!(
            matches!(location, Some("/first-run.html") | Some("/login.html")),
            "the pcap route must reach the auth layer's redirect, got {location:?}"
        );

        let bare = Router::new().route(
            "/local-api/pcapDump/{id}",
            get(crate::node_manager::local_api::packet_analysis::pcap_dump),
        );
        let (status, _, _) =
            request_with_headers(bare.clone(), "/local-api/pcapDump/not-a-number", None).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a non-numeric id must fail Path<usize> decoding"
        );

        let (status, _, _) =
            request_with_headers(bare, "/local-api/pcapDump/18446744073709551615", None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a numeric id must decode and reach the handler"
        );
    }

    /// `/api/v3` is key-only BEFORE routing.
    ///
    /// The mount uses `.layer`, so the API's own JSON-404 fallback and its
    /// method-not-allowed responses sit inside the auth middleware: without a
    /// key, a registered path, an unknown path and a wrong method are
    /// indistinguishable — all three are the same JSON 401. With a key they
    /// separate into 500 (handler, no bus) / JSON 404 / 405.
    ///
    /// The shared runtime-config lock is held for the whole test: the verifier
    /// decides against the CURRENT config snapshot, so a test that rewrites the
    /// config in parallel would otherwise be able to turn these 401s into
    /// something else.
    #[tokio::test]
    async fn api_v3_mount_is_key_only_before_routing() {
        let _config_guard = config_test_lock();
        let router = test_router();

        // Registered path, no key.
        let (status, body) = request(router.clone(), "/api/v3/health", None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "API routes must be key-only"
        );
        assert!(
            body.contains("x-bearer"),
            "the 401 body must be the API's JSON error, got: {body}"
        );

        // Unknown path, no key: same answer, so paths cannot be enumerated.
        let (status, body) = request(router.clone(), "/api/v3/no-such-endpoint", None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "an unknown API path must not be distinguishable without a key"
        );
        assert!(
            body.contains("x-bearer"),
            "the unknown-path 401 body must be JSON, got: {body}"
        );

        // Wrong method on a registered path, no key: still the auth boundary.
        let (status, _) = post(router.clone(), "/api/v3/health", None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a wrong method must not be distinguishable without a key"
        );

        // A cookie session (the admin UI's authority) must not open the API.
        let (status, _) = request_with_cookie(router.clone(), "/api/v3/health").await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "cookie sessions must not authorise /api/v3"
        );

        // The node manager's own surface is unchanged by the mount.
        let (status, body) = request(router, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "OK");
    }

    /// The API documents itself outside the key boundary.
    ///
    /// The spec and the Swagger UI are merged BESIDE the authenticated
    /// `/api/v3` nest, so both answer without a key while `/api/v3/health` still
    /// refuses. The docs surface is mounted outside `template.rs`, so it stamps
    /// the node-manager CSP itself — with `frame-ancestors 'self'`, because
    /// `api_docs.html` frames it.
    ///
    /// The slashless UI path is the crate's redirect to `/docs/`: the vendored
    /// page loads its assets with relative URLs, so a document served at
    /// `/docs` itself would resolve them against `/api/v3/` and render empty.
    #[tokio::test]
    async fn api_v3_docs_are_public_and_carry_the_csp() {
        let router = test_router();
        let expected_csp =
            crate::node_manager::security_headers::api_docs_content_security_policy();

        // The spec, without a key.
        let (status, headers, body) =
            request_with_headers(router.clone(), "/api/v3/openapi.json", None).await;
        assert_eq!(status, StatusCode::OK, "the spec must be public");
        assert_eq!(
            headers
                .get("content-security-policy")
                .and_then(|value| value.to_str().ok()),
            Some(expected_csp.as_str()),
            "the spec must carry the node-manager CSP"
        );
        let document: serde_json::Value =
            serde_json::from_str(&body).expect("the served spec must be JSON");
        assert_eq!(
            document["servers"][0]["url"],
            lqos_api::web::API_MOUNT_PREFIX,
            "the served spec must advertise the mount it is served from"
        );

        // The UI: redirect, then the page and one of its vendored assets.
        let (status, headers, _) = request_with_headers(router.clone(), "/api/v3/docs", None).await;
        assert_eq!(
            status,
            StatusCode::SEE_OTHER,
            "the slashless docs path is the crate's redirect to the UI mount"
        );
        assert_eq!(
            headers
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some("/api/v3/docs/")
        );

        let (status, headers, body) =
            request_with_headers(router.clone(), "/api/v3/docs/", None).await;
        assert_eq!(status, StatusCode::OK, "the UI must be public");
        assert_eq!(
            headers
                .get("content-security-policy")
                .and_then(|value| value.to_str().ok()),
            Some(expected_csp.as_str()),
            "the UI must carry the node-manager CSP"
        );
        assert!(
            body.contains("swagger-ui"),
            "the mounted page must be the Swagger UI, got: {body}"
        );

        let (status, _, _) =
            request_with_headers(router.clone(), "/api/v3/docs/swagger-ui.css", None).await;
        assert_eq!(status, StatusCode::OK, "the UI's vendored assets must load");

        // The data surface keeps its boundary.
        let (status, _, _) = request_with_headers(router, "/api/v3/health", None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "documentation is public; data is not"
        );
    }

    /// A configured key must pass the auth boundary, and a revoke must land on
    /// the next request.
    ///
    /// The keyed legs assert EXACT statuses, so the positive leg can fail. Which
    /// status `/api/v3/health` answers depends on the host: it asks the bus, and
    /// this test asserts the branch the host is actually in. The bus is probed
    /// first — a reachable bus means the handler runs to completion (200), no
    /// bus means the handler's own 500 with its bus-failure text (not 401, which
    /// would mean the key was rejected).
    #[tokio::test]
    async fn api_v3_accepts_a_configured_key_and_revokes_immediately() {
        let config = ApiKeyConfig::new("api-v3-key");
        let router = test_router();

        let (status, _) = request(router.clone(), "/api/v3/health", Some("lqos_api_wrong")).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "an unknown key must be rejected"
        );

        // Keyed, registered path: the handler runs, against whatever bus this
        // host has. The probe's client is dropped before the request so the two
        // never share a connection.
        let bus_reachable = lqos_bus::LibreqosBusClient::new().await.is_ok();
        let (status, body) =
            request(router.clone(), "/api/v3/health", Some(config.api_key())).await;
        if bus_reachable {
            assert_eq!(
                status,
                StatusCode::OK,
                "a configured key must reach the handler; this host has a bus, so it must answer 200, got: {body}"
            );
        } else {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "a configured key must reach the handler; no bus => 500, got: {body}"
            );
            assert!(
                body.contains("Failed to connect to bus"),
                "the 500 must be the handler's bus-failure body, got: {body}"
            );
        }

        // Keyed, unknown path: now the mount's JSON 404 answers.
        let (status, body) = request(
            router.clone(),
            "/api/v3/no-such-endpoint",
            Some(config.api_key()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            body.contains("unknown API path"),
            "unknown API paths must be JSON, not the static HTML fallback: {body}"
        );

        // Keyed, wrong method: axum's method-not-allowed answer (405), not a
        // 404 and not the auth boundary.
        let (status, _) = post(router.clone(), "/api/v3/health", Some(config.api_key())).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "a wrong method on a registered path must be 405 once authenticated"
        );

        config.revoke_all();
        let (status, _) = request(router, "/api/v3/health", Some(config.api_key())).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a revoke must take effect on the next request"
        );
    }

    /// Retired paths are gone from the mount, and the boundary still holds.
    ///
    /// This is the campaign's subtraction guard: every route an earlier unit
    /// deleted (the Insight `/lts` surface, the bakery/byte-counter arms, the
    /// chatbot and support-ticket write paths, the raw-queue snapshot, the v2
    /// site-speed writes) must answer nothing. Without a key the mount cannot be
    /// enumerated: all of them are the same JSON 401. With a key they are the
    /// mount's JSON 404 — not a handler's error page, not the static fallback.
    #[tokio::test]
    async fn retired_api_v3_paths_are_json_404_with_a_key_and_401_without() {
        let config = ApiKeyConfig::new("api-v3-retired-paths");
        let router = test_router();

        let retired = [
            "/api/v3/lts/status",
            "/api/v3/chatbot/message",
            "/api/v3/support_ticket",
            "/api/v3/bakery_status",
            "/api/v3/bakery_stats",
            "/api/v3/byte_counters",
            "/api/v3/mapped_ips",
            "/api/v3/clear_hot_cache",
            "/api/v3/raw_queue_data/circuit-1",
            "/api/v3/network_json/set_site_speed",
            "/api/v3/network_json/set_site_speed_batch",
        ];

        for path in retired {
            let (status, body) = request(router.clone(), path, None).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} must stay behind the key boundary"
            );
            assert!(
                body.contains("x-bearer"),
                "{path} must answer the API's JSON 401, got: {body}"
            );

            let (status, body) = request(router.clone(), path, Some(config.api_key())).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{path} is retired and must not route, got: {body}"
            );
            assert!(
                body.contains("unknown API path"),
                "{path} must be the mount's JSON 404, not a handler's answer: {body}"
            );
        }
    }

    /// The verifier fails CLOSED when the config cannot be read.
    ///
    /// `require_api_key` decides against the current config snapshot; if that
    /// snapshot cannot be loaded, the only safe answer is the JSON 401 — never a
    /// pass-through, never a panic. The key here is one no snapshot would accept,
    /// so a permissive fallback would be visible as a non-401 answer.
    #[tokio::test]
    async fn api_v3_verifier_fails_closed_without_a_readable_config() {
        let _missing = UnreadableConfig::new("fail-closed");
        let router = test_router();

        let (status, body) = request(
            router,
            "/api/v3/health",
            Some("lqos_api_key_that_no_snapshot_accepts"),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "an unreadable config must not open the API"
        );
        assert!(
            body.contains("x-bearer"),
            "the refusal must be the API's JSON 401, got: {body}"
        );
    }

    /// The shared runtime-config lock, for tests that depend on (or rewrite) the
    /// config snapshot. Poisoning is recovered: the fixtures restore what they
    /// change, so one failing test must not cascade into every later one.
    fn config_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::runtime_config_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Points `LQOS_CONFIG` at a path that does not exist, under the shared
    /// runtime-config lock, and restores the previous value on drop.
    struct UnreadableConfig {
        _guard: std::sync::MutexGuard<'static, ()>,
        previous: Option<std::ffi::OsString>,
    }

    impl UnreadableConfig {
        fn new(label: &str) -> Self {
            let guard = config_test_lock();
            let missing = std::env::temp_dir()
                .join(format!("lqosd-{label}-{}", std::process::id()))
                .join("missing.conf");
            let previous = std::env::var_os("LQOS_CONFIG");
            unsafe { std::env::set_var("LQOS_CONFIG", &missing) };
            lqos_config::clear_cached_config();
            Self {
                _guard: guard,
                previous,
            }
        }
    }

    impl Drop for UnreadableConfig {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            lqos_config::clear_cached_config();
        }
    }

    /// The shipped example config with both directory keys pointed at `dir`, so a
    /// fixture writes only the section it is about.
    fn example_config(dir: &Path) -> String {
        include_str!("../../../lqos_config/src/etc/v15/example.toml")
            .replacen(
                "lqos_directory = \"/opt/libreqos/src\"",
                &format!("lqos_directory = {:?}", dir.to_string_lossy()),
                1,
            )
            .replacen(
                "state_directory = \"/opt/libreqos/state\"",
                &format!(
                    "state_directory = {:?}",
                    dir.join("state").to_string_lossy()
                ),
                1,
            )
    }

    /// One temp runtime directory plus the `lqos.conf` the daemon reads, held
    /// under the shared runtime-config test lock so parallel tests never see a
    /// half-written config.
    ///
    /// `configure` receives the temp directory (already carrying `state/`) and
    /// returns the raw config text; it may also seed the served static tree. For
    /// its lifetime `LQOS_CONFIG` and `LQOS_DIRECTORY` point at the temp tree and
    /// the config cache is dropped on the way in and on the way out: both are
    /// process globals every `load_config()` caller shares.
    struct TempLqosConfig {
        _guard: std::sync::MutexGuard<'static, ()>,
        dir: std::path::PathBuf,
        config_path: std::path::PathBuf,
        old_lqos_config: Option<std::ffi::OsString>,
        old_lqos_directory: Option<std::ffi::OsString>,
    }

    impl TempLqosConfig {
        fn new(label: &str, configure: impl FnOnce(&Path) -> String) -> Self {
            let guard = config_test_lock();
            let dir = std::env::temp_dir().join(format!("lqosd-{label}-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("state")).expect("create runtime dir");
            let config_path = dir.join("lqos.conf");
            std::fs::write(&config_path, configure(&dir)).expect("write test config");

            let old_lqos_config = std::env::var_os("LQOS_CONFIG");
            let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &dir);
            }
            lqos_config::clear_cached_config();
            Self {
                _guard: guard,
                dir,
                config_path,
                old_lqos_config,
                old_lqos_directory,
            }
        }

        fn config_path(&self) -> &Path {
            &self.config_path
        }
    }

    impl Drop for TempLqosConfig {
        fn drop(&mut self) {
            match &self.old_lqos_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_lqos_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A temp `lqos.conf` carrying one `[local_api]` key.
    struct ApiKeyConfig {
        temp: TempLqosConfig,
        api_key: String,
    }

    impl ApiKeyConfig {
        fn new(label: &str) -> Self {
            // Mint the record through the SAME seam the key manager uses, so the
            // fixture cannot drift from the real digest.
            let (record, creation) = crate::node_manager::local_api::local_api_keys::build_key(
                &lqos_config::Config::default(),
                "ext",
                uuid::Uuid::nil(),
                &[7_u8; crate::node_manager::local_api::local_api_keys::KEY_RANDOM_BYTES],
                1,
            )
            .expect("mint a test API key");
            let api_key = creation.api_key;

            let temp = TempLqosConfig::new(label, |dir| {
                example_config(dir).replacen(
                    "bearer_token = \"\"",
                    &format!(
                        "bearer_token = \"\"\n\n[[local_api.keys]]\nid = \"{}\"\nname = \"{}\"\ntoken_sha256 = \"{}\"\ncreated_at_unix = {}",
                        record.id,
                        record.name,
                        record.token_sha256,
                        record.created_at_unix,
                    ),
                    1,
                )
            });
            Self { temp, api_key }
        }

        fn api_key(&self) -> &str {
            &self.api_key
        }

        /// Drops every key from the config file and invalidates the snapshot, the
        /// way `update_config` publishes a revoke.
        fn revoke_all(&self) {
            let raw = std::fs::read_to_string(self.temp.config_path()).expect("read test config");
            let start = raw.find("[[local_api.keys]]").expect("key block");
            let end = raw.find("[ip_ranges]").expect("section after local_api");
            let revoked = format!("{}{}", &raw[..start], &raw[end..]);
            std::fs::write(self.temp.config_path(), revoked).expect("write revoked config");
            lqos_config::clear_cached_config();
        }
    }

    fn test_router() -> Router {
        test_router_with(api_state())
    }

    fn test_router_with(api_state: std::sync::Arc<lqos_api::api::ApiState>) -> Router {
        let static_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node_manager/static2");
        let (bus_tx, _bus_rx) = tokio::sync::mpsc::channel(8);
        let (system_usage_tx, _system_usage_rx) = crossbeam_channel::bounded(8);
        let (control_tx, _control_rx) = tokio::sync::mpsc::channel(8);
        let (shaper_tx, _shaper_rx) = tokio::sync::mpsc::channel(8);
        let probe_client =
            lqos_probe::ProbeManager::spawn(lqos_probe::ProbeManagerConfig::default());

        build_router(
            &static_path,
            bus_tx,
            system_usage_tx,
            control_tx,
            probe_client,
            shaper_tx,
            api_state,
        )
        .expect("node manager router should build from the in-tree static2 tree")
    }

    async fn request(router: Router, path: &str, bearer: Option<&str>) -> (StatusCode, String) {
        let (status, _, body) = request_with_headers(router, path, bearer).await;
        (status, body)
    }

    /// Sends a `GET`, keeping the response headers for the assertions that need
    /// them (the docs CSP and its redirect target).
    async fn request_with_headers(
        router: Router,
        path: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut builder = axum::http::Request::builder().uri(path);
        if let Some(bearer) = bearer {
            builder = builder.header("x-bearer", bearer);
        }
        send(router, builder, &[]).await
    }

    async fn post(router: Router, path: &str, bearer: Option<&str>) -> (StatusCode, String) {
        let mut builder = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri(path);
        if let Some(bearer) = bearer {
            builder = builder.header("x-bearer", bearer);
        }
        let (status, _, body) = send(router, builder, &[]).await;
        (status, body)
    }

    async fn request_with_cookie(router: Router, path: &str) -> (StatusCode, String) {
        let builder = axum::http::Request::builder()
            .uri(path)
            .header("cookie", "User-Token=admin-session");
        let (status, _, body) = send(router, builder, &[]).await;
        (status, body)
    }

    /// The one request round trip: every request helper in this module ends
    /// here.
    ///
    /// `headers` are extra field lines (including repeated names such as two
    /// `accept-language` lines): a request builder cannot carry them all through
    /// the callers that vary exactly one header.
    async fn send(
        router: Router,
        mut builder: axum::http::request::Builder,
        headers: &[(&str, &str)],
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        use tower::ServiceExt;

        // Generous on purpose: the served spec is ~400 KB and the vendored
        // Swagger UI bundle ~1.4 MB, so a tight cap would fail on the docs
        // surface rather than on a wrong answer.
        const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = router
            .oneshot(builder.body(axum::body::Body::empty()).expect("request"))
            .await
            .expect("router should answer");
        let status = response.status();
        let response_headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), MAX_BODY_BYTES)
            .await
            .expect("body");
        (
            status,
            response_headers,
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    /// Temp config plus the served static tree the node manager's own template
    /// path reads (`<lqos_directory>/bin/static2`). The shell and the two
    /// standalone pages are copied from the in-tree `static2`, so the render
    /// runs against the real placeholders.
    struct UiLocaleFixture {
        /// Held for its `Drop`: restoring the environment and deleting the temp
        /// tree is the fixture's whole remaining job.
        _temp: TempLqosConfig,
    }

    impl UiLocaleFixture {
        /// `config_language` writes `ui_language` at the top of the config;
        /// `with_user` creates the first admin so authenticated pages answer
        /// `200`.
        fn new(label: &str, config_language: Option<&str>, with_user: bool) -> Self {
            let label = format!("ui-locale-{label}");
            let temp = TempLqosConfig::new(&label, |dir| {
                let static_dir = dir.join("bin/static2");
                std::fs::create_dir_all(&static_dir).expect("create static dir");
                let source_static =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node_manager/static2");
                for page in ["template.html", "login.html", "first-run.html"] {
                    std::fs::copy(source_static.join(page), static_dir.join(page))
                        .unwrap_or_else(|e| panic!("copy {page}: {e}"));
                }
                let raw = example_config(dir);
                match config_language {
                    Some(tag) => format!("ui_language = {tag:?}\n{raw}"),
                    None => raw,
                }
            });
            crate::node_manager::auth::invalidate_auth_cache();

            if with_user {
                let mut users =
                    lqos_config::WebUsers::load_or_create().expect("create the users file");
                users
                    .add_or_update_user("admin", "secret", lqos_config::UserRole::Admin)
                    .expect("add the test admin");
                crate::node_manager::auth::invalidate_auth_cache();
            }

            Self { _temp: temp }
        }
    }

    impl Drop for UiLocaleFixture {
        fn drop(&mut self) {
            // The fields restore the environment after this runs, so drop the
            // cached auth snapshot while it still describes this fixture.
            crate::node_manager::auth::invalidate_auth_cache();
        }
    }

    /// Logs in through the router and returns the `User-Token` cookie pair.
    async fn login_cookie(router: &Router) -> String {
        use tower::ServiceExt;

        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/doLogin")
                    .header("content-type", "application/json")
                    .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                        [127, 0, 0, 1],
                        40000,
                    ))))
                    .body(axum::body::Body::from(
                        r#"{"username":"admin","password":"secret"}"#,
                    ))
                    .expect("login request"),
            )
            .await
            .expect("login answers");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the test admin must be able to log in"
        );
        response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("login sets a session cookie")
            .to_str()
            .expect("session cookie is ASCII")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string()
    }

    /// A `GET` for `path` carrying `cookies` as one `Cookie` field line.
    fn page_request(path: &str, cookies: &[&str]) -> axum::http::request::Builder {
        let builder = axum::http::Request::builder().uri(path);
        if cookies.is_empty() {
            builder
        } else {
            builder.header("cookie", cookies.join("; "))
        }
    }

    /// One templated request nests deep enough that a debug build overflows the
    /// default 2 MiB test stack, so these tests run on a thread with room (the
    /// daemon sets its own stack size for the same reason).
    fn on_wide_stack(body: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(body)
            .expect("spawn the wide-stack test thread")
            .join()
            .expect("the wide-stack test thread panicked");
    }

    fn block_on(future: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build the test runtime")
            .block_on(future);
    }

    /// The three substitutions the operator-facing contract names: the
    /// `Content-Language` header, the `lang`/`dir` attributes and the catalog
    /// script tags, with no template token left behind. `dir` is the direction
    /// the served locale's catalog record declares.
    fn assert_locale_render(lang: &str, dir: &str, headers: &axum::http::HeaderMap, body: &str) {
        assert_eq!(
            headers
                .get(axum::http::header::CONTENT_LANGUAGE)
                .and_then(|value| value.to_str().ok()),
            Some(lang),
            "Content-Language must name the resolved locale"
        );
        assert!(
            body.contains(&format!(r#"<html lang="{lang}" dir="{dir}""#)),
            "the page must carry lang/dir: {body:.200}"
        );
        // The English catalog always loads: the runtime reads its fallback
        // strings from it, so a page that loaded only the served catalog would
        // render untranslated keys as their key names. The shared helper owns
        // what counts as a catalog; every page's generated locale list is
        // asserted in `template.rs`.
        let scripts = crate::node_manager::template::catalog_scripts(body);
        if lang == "en" {
            assert_eq!(scripts, ["en.js"], "English is its own fallback");
        } else {
            assert_eq!(
                scripts,
                ["en.js", format!("{lang}.js").as_str()],
                "one fallback catalog, then the served one"
            );
        }
        assert!(
            !body.contains("%%"),
            "no template token may survive: {body:.200}"
        );
    }

    /// A negotiated page may not be stored or replayed by a shared cache.
    fn assert_not_cacheable(headers: &axum::http::HeaderMap) {
        assert_eq!(
            headers
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store"),
            "a negotiated page must not be stored"
        );
        let vary = headers
            .get(axum::http::header::VARY)
            .and_then(|value| value.to_str().ok())
            .expect("a negotiated page must name its Vary fields")
            .to_ascii_lowercase();
        for field in ["cookie", "accept-language"] {
            assert!(vary.contains(field), "Vary must name {field}: {vary}");
        }
    }

    #[test]
    fn templated_page_resolves_the_cookie_then_accept_language() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("cookie", None, true);
                let router = test_router();
                let session = login_cookie(&router).await;

                let (status, headers, body) = send(
                    router.clone(),
                    page_request("/help.html", &[session.as_str(), "lqos_lang=es"]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
                // The shell is composed per request (role, status, locale); it
                // carries no `Vary` because `no-store` already keeps every cache
                // out of it.
                assert_eq!(
                    headers
                        .get(axum::http::header::CACHE_CONTROL)
                        .and_then(|value| value.to_str().ok()),
                    Some("no-store"),
                    "a composed page must not be stored"
                );

                let (status, headers, body) = send(
                    router.clone(),
                    page_request("/help.html", &[session.as_str()]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("en", "ltr", &headers, &body);

                let (status, headers, body) = send(
                    router,
                    page_request("/help.html", &[session.as_str()]),
                    &[("accept-language", "es-MX,es;q=0.9,en;q=0.8")],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
            })
        });
    }

    #[test]
    fn config_language_is_the_node_default_and_the_cookie_wins() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("config", Some("es"), true);
                let router = test_router();
                let session = login_cookie(&router).await;

                let (status, headers, body) = send(
                    router.clone(),
                    page_request("/help.html", &[session.as_str()]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);

                let (status, headers, body) = send(
                    router,
                    page_request("/help.html", &[session.as_str(), "lqos_lang=en"]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("en", "ltr", &headers, &body);
            })
        });
    }

    #[test]
    fn unknown_config_language_counts_as_absent() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("config-unknown", Some("de"), true);
                let router = test_router();
                let session = login_cookie(&router).await;

                let (status, headers, body) = send(
                    router,
                    page_request("/help.html", &[session.as_str()]),
                    &[("accept-language", "es;q=0.9")],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
            })
        });
    }

    /// A sender may split `Accept-Language` across field lines; both lines are
    /// one list, so the second must still be read.
    #[test]
    fn split_accept_language_field_lines_are_joined() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("split", None, true);
                let router = test_router();
                let session = login_cookie(&router).await;

                let (status, headers, body) = send(
                    router,
                    page_request("/help.html", &[session.as_str()]),
                    &[("accept-language", "de"), ("accept-language", "es")],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
            })
        });
    }

    #[test]
    fn standalone_first_run_page_carries_the_locale_substitutions() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("standalone", None, false);
                let router = test_router();

                let (status, headers, body) = send(
                    router.clone(),
                    page_request("/first-run.html", &[]),
                    &[("accept-language", "es-MX,es;q=0.9")],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
                assert_not_cacheable(&headers);

                // The control this page carries writes `lqos_lang`; the reload
                // that follows is a fresh request, so the server has to resolve
                // that cookie into the same language.
                let (status, headers, body) = send(
                    router,
                    page_request("/first-run.html", &["lqos_lang=es"]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
            })
        });
    }

    /// The page every unauthenticated operator sees gets the same substitutions,
    /// negotiated from the same headers: the fixture has users, but no session.
    #[test]
    fn standalone_login_page_carries_the_locale_substitutions() {
        on_wide_stack(|| {
            block_on(async {
                let _fixture = UiLocaleFixture::new("login", None, true);
                let router = test_router();

                let (status, headers, body) = send(
                    router.clone(),
                    page_request("/login.html", &[]),
                    &[("accept-language", "es-MX,es;q=0.9")],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
                assert_not_cacheable(&headers);

                // Same round trip as the first-run page: the control's cookie,
                // not the browser's headers, decides the composing language.
                let (status, headers, body) = send(
                    router,
                    page_request("/login.html", &["lqos_lang=es"]),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_locale_render("es", "ltr", &headers, &body);
            })
        });
    }
}
