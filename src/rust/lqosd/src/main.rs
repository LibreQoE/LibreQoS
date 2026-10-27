//! `lqosd` is the core of LibreQoS. It runs as a daemon, loads the XDP,
//! manages TC creation, and provides the web interface.

#![deny(clippy::unwrap_used)]

mod blackboard;
pub mod bridge_analytics;
pub mod bridge_broker;
pub mod bridge_supervisor;
mod dynamic_circuits;
mod e2_rates;
mod file_lock;
mod heimdall;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
pub mod lts2_sys;
mod memory_watchdog;
pub mod netflow;
mod network_devices_hooks;
mod node_manager;
mod outage_containment;
mod probe_provider;
mod program_control;
mod radius_accounting;
mod remote_commands;
mod rtt_exclusions;
mod scheduler_control;
mod shaped_devices_tracker;
mod shaping_runtime;
mod stats;
mod system_stats;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod k3_retirement_gate;
mod throughput_tracker;
mod tool_status;
mod tunnel_circuits_ingest;
mod tuning;
mod urgent;
mod validation;
mod version_checks;

#[cfg(feature = "flamegraphs")]
use std::io::Write;
use std::net::IpAddr;

use crate::{
    file_lock::FileLock,
    throughput_tracker::flow_data::live_active_flow_count,
};
use crate::blackboard::{BLACKBOARD_SENDER, BlackboardCommand};
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse, InsightLicenseSummary, UnixSocketServer};
use lqos_heimdall::{n_second_packet_dump, start_heimdall};
use lqos_utils::rustls::ensure_rustls_crypto_provider;
use lqos_utils::unix_time::unix_now;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use stats::{BUS_REQUESTS, TIME_TO_POLL_HOSTS};
use std::time::Duration;
use tracing::{error, info, warn};

// Use MiMalloc only on supported platforms
//#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
//use mimalloc::MiMalloc;

use lqos_stormguard::{STORMGUARD_DEBUG, STORMGUARD_STATS};
use tracing::level_filters::LevelFilter;
// Use MiMalloc only on supported platforms
// #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
// #[global_allocator]
// static GLOBAL: MiMalloc = MiMalloc;

/// The stock default filter: WARN, plus the dedicated LTS-volume diagnostic
/// target at INFO (`lqos-74t.2.1.1`). The `.2.1` LTS-volume instruments are
/// `info!` on the `lqos_lts_volume` target, so a fresh box (no `RUST_LOG`) can
/// read the LTS volume without a drop-in while nothing else floods.
pub fn default_filter_directives() -> &'static str {
    "warn,lqos_lts_volume=info"
}

/// Configure a highly detailed logging system.
pub fn set_console_logging() -> anyhow::Result<()> {
    // `RUST_LOG`, when set, is honored as a BARE level (the historical
    // behavior); any other value falls to WARN. When it is UNSET, the stock
    // default keeps WARN but lifts the LTS-volume target to INFO.
    let fmt = tracing_subscriber::fmt()
        // Use a more compact, abbreviated log format
        .compact()
        // Display source code file paths
        .with_file(true)
        // Display source code line numbers
        .with_line_number(true)
        // Display the thread ID an event was recorded on
        .with_thread_ids(false)
        // Don't display the event's target (module path)
        .with_target(false);

    match std::env::var("RUST_LOG").ok() {
        Some(level) => {
            let level = match level.to_lowercase().as_str() {
                "trace" => LevelFilter::TRACE,
                "debug" => LevelFilter::DEBUG,
                "info" => LevelFilter::INFO,
                "warn" => LevelFilter::WARN,
                "error" => LevelFilter::ERROR,
                _ => LevelFilter::WARN,
            };
            tracing::subscriber::set_global_default(fmt.with_max_level(level).finish())?;
        }
        None => {
            // `env-filter` is enabled in the workspace; the default keeps WARN
            // and enables ONLY the LTS-volume diagnostic target at INFO.
            let filter = tracing_subscriber::EnvFilter::new(default_filter_directives());
            tracing::subscriber::set_global_default(fmt.with_env_filter(filter).finish())?;
        }
    }
    Ok(())
}

fn initialize_license_storage(initialize_storage: impl FnOnce() -> Result<()>) {
    info!("Starting local Insight license-cache initialization");
    match initialize_storage() {
        Ok(()) => info!("Completed local Insight license-cache initialization successfully"),
        Err(err) => record_shaping_warning(
            "Insight license-cache initialization completed with an error",
            format!("{err:?}"),
        ),
    }
}

/// Surfaces a recoverable subsystem failure: logs it, records it in the
/// operator warning sink, and latches the shaping runtime state. One composite
/// so the three writer calls stay in a single place (not scattered per site).
fn record_shaping_failure(
    state: shaping_runtime::ShapingRuntimeState,
    summary: impl Into<String>,
    detail: impl Into<String>,
) {
    let summary = summary.into();
    let detail = detail.into();
    error!("{summary}: {detail}");
    node_manager::add_global_warning(node_manager::WarningLevel::Error, summary.clone());
    shaping_runtime::mark_error(state, summary, detail);
}

/// Surfaces a recoverable warning that does not by itself degrade the shaping
/// runtime state; the operator still sees it in the warning sink.
fn record_shaping_warning(summary: impl Into<String>, detail: impl Into<String>) {
    let summary = summary.into();
    let detail = detail.into();
    warn!("{summary}: {detail}");
    node_manager::add_global_warning(node_manager::WarningLevel::Warning, summary);
}

fn main() -> Result<()> {
    // The seam-align probe (the v2->v3 preflight / deb preinst): print the
    // wire PROTOCOL_VERSION and exit, so an installed lqosd's protocol can be
    // compared against the new dataplane artifact's (the mixed-version hazard
    // that presents as a bridge "magic/version mismatch").
    if std::env::args().any(|arg| arg == "--print-protocol-version") {
        println!("{}", lqos_dataplane_types::wire::PROTOCOL_VERSION);
        return Ok(());
    }
    // #4 (`docs/resolved_posture_spec.md`): the resolved-posture probe — print
    // the RESOLVED semantic posture and exit (mirrors --print-protocol-version),
    // so the DoD `diff <(lqosd --print-resolved-posture|sort) ...` is mechanical.
    if std::env::args().any(|arg| arg == "--print-resolved-posture") {
        use crate::bridge_broker::posture::{PostureInput, render_probe, resolve_posture};
        let input = match lqos_config::load_config() {
            Ok(config) => PostureInput {
                queue_mode_observe: config.queues.queue_mode.is_observe(),
                dataplane_bridge_present: config.dataplane_bridge.is_some(),
            },
            Err(_) => PostureInput {
                queue_mode_observe: false,
                dataplane_bridge_present: false,
            },
        };
        print!("{}", render_probe(&resolve_posture(input)));
        return Ok(());
    }

    // WP5 (`docs/wp5_seam_state_surface.md`): the reconciliation-state
    // probe — print the PERSISTED `SEAM_STATE` snapshot and exit. LEVEL-INDEPENDENT:
    // lqosd defaults to WARN, so the `info!` emit never reaches the journal at the
    // default; this reads the file the broker's tick writes. Diffable + one-command.
    if std::env::args().any(|arg| arg == "--print-seam-state") {
        match std::fs::read_to_string("/run/libreqos/seam_state.json") {
            Ok(json) => print!("{json}"),
            Err(_) => println!("SEAM_STATE unavailable (the broker has not ticked yet)"),
        }
        return Ok(());
    }

    // Set up logging
    set_console_logging()?;

    // Configure glibc resolver defaults so DNS lookups bound quickly.
    // If the user hasn't set RES_OPTIONS, set a conservative timeout/attempts.
    if std::env::var_os("RES_OPTIONS").is_none() {
        // 2s per attempt, 1 attempt keeps worst-case DNS stalls short
        // Safety: Environment variable is unsafe because it modifies global state.
        unsafe {
            std::env::set_var("RES_OPTIONS", "timeout:2 attempts:1");
        }
    }

    // Check that the file lock is available. Bail out if it isn't.
    let file_lock = FileLock::new().inspect_err(|e| {
        error!("Unable to acquire file lock: {:?}", e);
        std::process::exit(0);
    })?;

    // Announce startup
    info!("LibreQoS Daemon Starting");

    // Load config
    let config = lqos_config::load_config()?;
    // Phase D: the explicit repair entry — lqosd startup is the PRIMARY owner.
    // It runs BEFORE the runtime's first load; a failure is a LOUD boot failure
    // (fail-closed), never a degraded silent start.
    lqos_config::repair_runtime_state(config.as_ref())
        .map_err(|e| anyhow::anyhow!("runtime-state repair failed at startup: {e}"))?;
    // `lqos-74t.118.3`: compute the shaping-CPU topology once at startup for
    // the same-origin CPU & Workers page (never on the request path).
    node_manager::local_api::cpu_workers::init_topology(config.as_ref());
    let web_config = config.clone();
    let radius_accounting_config = config.radius_accounting.clone();
    shaping_runtime::mark_starting("LibreQoS is starting shaping services.");

    ensure_rustls_crypto_provider()?;

    initialize_license_storage(|| lts2_sys::license_grant::init_license_storage(&config));

    // Apply Tunings
    tuning::tune_lqosd_from_config_file()?;

    // Spawn tracking sub-systems
    // Fatal: the daemon exits before the API is serving, so there is no live
    // surface to carry this failure.
    let Ok(control_channel) = lts2_sys::control_channel::init_control_channel() else {
        error!("Failed to initialize Insight control channel, exiting.");
        std::process::exit(1);
    };
    let control_tx_for_lts = control_channel.tx.clone();
    let control_tx_for_web = control_channel.tx.clone();
    if let Err(e) = lts2_sys::start_lts2(control_tx_for_lts) {
        record_shaping_failure(
            shaping_runtime::ShapingRuntimeState::Degraded,
            "Insight client failed to start",
            format!("{e:?}"),
        );
    } else {
        info!("Insight client started successfully");
    }
    lqos_network_devices::start_daemon_mode(Some(std::sync::Arc::new(
        network_devices_hooks::LqosdNetworkDevicesHooks,
    )))?;
    // The system-usage actor is started BEFORE the bridge analytics actor so
 // its sender can ride the LTS/Insight submission trigger.
    let system_usage_tx = system_stats::start_system_stats()?;
    // E1 (bridge broker): owns the bridge control seam — watches the
    // published dataplane transaction and emits semantic deltas.
    // Inert under queue_mode=observe.
    bridge_broker::start_bridge_broker(&config);
    // E2 (stats + health): bridge analytics polling + the Supervisor
    // probe. Inert without [dataplane_bridge] / the analytics socket
    // env (kernel-dataplane installs untouched). The system-usage sender
    // powers the orphaned LTS submission trigger.
    bridge_analytics::start_bridge_analytics(&config, system_usage_tx.clone());
    bridge_supervisor::start_bridge_supervisor(&config);
    // C2 (circuit-outage containment): the cold-path black-hole + churn
    // tripwire — ONE operator-visible surface that names a held circuit.
    outage_containment::start_outage_containment();
    // 3a: the NetFlow exporter (F-A export-as-is; a cold-path consumer of the
    // retained bridge completion events; a no-op unless [flows] enables it).
    netflow::start(&config);
    // 3a LTS reinstatement (the #32 spawn-move pattern): the Insight blackboard
    // relay and the remote-command worker are attach-orphans — spawn them on
    // the common startup path (their only former spawner was the deleted
    // kernel attach).
    blackboard::start_blackboard();
    remote_commands::start_remote_commands();

    // Handle signals
    let mut signals = Signals::new([SIGINT, SIGHUP, SIGTERM])?;
    // Memory Debugging
    memory_debug();
    memory_watchdog::start_memory_watchdog();

    // Prepare the bus path and clear any stale socket before fatal attach checks.
    // The bus does not bind or listen until after shaping startup below.
    let server = UnixSocketServer::new()
        .map_err(|err| anyhow::anyhow!("Unable to spawn bus socket server: {err}"))?;

    version_checks::start_version_check()?;

    // K3-A: the XDP/TC kernel attach path is gone — LibreQoS is bridge-only.
    // There is no kernel shaper sender, so StormGuard starts with `None` and
    // runs its bridge arms. K3-B retires the kernel flowbee feed entirely;
    // bridge flow history is the flow-analysis source.
 // K2 defect-fix: both file watchers are mode-neutral.
    // The kernel-attach branch never runs on a bridge-only box, so spawning
    // them there left bridge deployments without the E2 effective-rate refresh
    // (topology_runtime_status.json) and without the tunnel_circuits.json
    // cold-pass ingest. Spawn them on the common startup path instead: kernel
    // boxes are unchanged and bridge boxes gain both.
    shaped_devices_tracker::topology_runtime_status_watcher()?;
    tunnel_circuits_ingest::start_tunnel_circuits_watcher()?;
    // K2 slice 1: the ASN GeoTable is mode-neutral data (the TopFlows ASN
    // join needs it on bridge boxes too); only the kernel flowbee feed stays
    // kernel-gated.
    let _ = throughput_tracker::flow_data::setup_flow_analysis();

    let control_tx_for_webserver = control_tx_for_web.clone();
    let system_usage_tx_for_web = system_usage_tx.clone();
    // K1b startup decoupling §1.5: a bridge-only box IS the shaper, so report
    // Active when the kernel path started OR the bridge analytics are live.
    let shaping_started = bridge_analytics::is_active();
    // K3-A: the kernel transport is gone. The bridge analytics socket is the
    // only capture transport; a box without bridge analytics has no capture
    // feed and is left without Heimdall.
    let heimdall_transport = if bridge_analytics::is_active() {
        let settings = config.dataplane_bridge.clone().unwrap_or_default();
        Some(lqos_heimdall::HeimdallTransport::Bridge {
            socket: bridge_analytics::analytics_socket_path(&settings),
        })
    } else {
        None
    };
    match heimdall_transport {
        Some(transport) => start_heimdall(transport)?,
        None => tracing::debug!("Heimdall inactive: no kernel attach and no bridge analytics"),
    }
    let handle = std::thread::Builder::new()
        .name("Async Bus/Web".to_string())
        .spawn(move || {
            let Ok(tokio_runtime) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 * 1024 * 1024)
                .build()
            else {
                // Fatal: the bus/web thread returns here; without it there is
                // no API to surface through.
                error!("Unable to start Tokio runtime. Not much is going to work");
                return;
            };
            tokio_runtime.block_on(async {
                let probe_client =
                    lqos_probe::ProbeManager::spawn(lqos_probe::ProbeManagerConfig::default());
                let probe_client_for_stormguard = probe_client.clone();
                probe_provider::install_probe_client(probe_client.clone());

                // K1b §4.6 pin 3: StormGuard needs the in-process bus to poll
                // the bridge publish outcome ring, so the channel is created
                // before the StormGuard task (the topology thread and the
                // webserver take clones below).
                let (bus_tx, bus_rx) = tokio::sync::mpsc::channel::<(
                    tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
                    BusRequest,
                )>(100);
                let bus_tx_for_stormguard = bus_tx.clone();

                tokio::spawn(async move {
                    match lts2_sys::control_channel::start_control_channel(control_channel).await {
                        Ok(_) => info!("Insight control channel started successfully"),
                        Err(e) => record_shaping_failure(
                            shaping_runtime::ShapingRuntimeState::Degraded,
                            "Insight control channel failed to start",
                            format!("{e:#}"),
                        ),
                    }
                });

                if let Err(err) = radius_accounting::start_configured_radius_accounting(
                    radius_accounting_config,
                    &web_config,
                    bus_tx.clone(),
                )
                .await
                {
                    record_shaping_failure(
                        shaping_runtime::ShapingRuntimeState::Degraded,
                        "RADIUS accounting listener was not started",
                        format!("{err}"),
                    );
                }

                lqos_topology::start_topology_thread(bus_tx.clone());

                // K1b startup decoupling §1.1: StormGuard spawns AFTER the
                // topology thread and UNCONDITIONALLY; on a bridge-only box the
                // sender is None and the kernel/bakery arms no-op.
                tokio::spawn(async move {
                    use tokio::time::{Duration, sleep};
                    for _ in 0..100u32 {
                        if tokio::fs::metadata(lqos_bus::BUS_SOCKET_PATH).await.is_ok() {
                            break;
                        }
                        sleep(Duration::from_millis(50)).await;
                    }
                    match lqos_stormguard::start_stormguard(
                        shaped_devices_tracker::full_network_map_snapshot,
                        crate::bridge_analytics::bridge_passive_rtt_p90_by_name,
                        probe_client_for_stormguard,
                        bus_tx_for_stormguard,
                    )
                    .await
                    {
                        Ok(_) => info!("StormGuard started successfully"),
                        Err(e) => record_shaping_failure(
                            shaping_runtime::ShapingRuntimeState::Degraded,
                            "StormGuard failed to start",
                            format!("{e:#}"),
                        ),
                    }
                });

                let webserver_disabled = web_config.disable_webserver.unwrap_or(false);
                if !webserver_disabled {
                    let control_tx_for_webserver = control_tx_for_webserver.clone();
                    let probe_client_for_webserver = probe_client.clone();
                    tokio::spawn(async move {
                        if let Err(e) = node_manager::spawn_webserver(
                            bus_tx,
                            system_usage_tx_for_web,
                            control_tx_for_webserver,
                            probe_client_for_webserver,
                        )
                        .await
                        {
                            record_shaping_failure(
                                shaping_runtime::ShapingRuntimeState::Degraded,
                                "Node Manager (webserver) failed to start",
                                format!("{e:?}"),
                            );
                        }
                    });
                } else {
                    warn!("Webserver disabled by configuration");
                }

                if let Err(e) = server.listen(handle_bus_requests, bus_rx).await {
                    error!("Bus stopped: {e:?}");
                }
            });
        })?;
    if shaping_started {
        shaping_runtime::mark_active("LibreQoS shaping is active.");
    }

    std::thread::Builder::new()
        .name("Signal Handler".to_string())
        .spawn(move || {
            for sig in signals.forever() {
                match sig {
                    SIGINT | SIGTERM => {
                        match sig {
                            SIGINT => warn!("Terminating on SIGINT"),
                            SIGTERM => warn!("Terminating on SIGTERM"),
                            _ => {
                                warn!("This should never happen - terminating on unknown signal")
                            }
                        }
                        UnixSocketServer::signal_cleanup();
                        std::mem::drop(file_lock);
                        std::process::exit(0);
                    }
                    SIGHUP => {
                        warn!("Reloading configuration because of SIGHUP");
                        let result = tuning::tune_lqosd_from_config_file();
                        if let Err(err) = result {
                            warn!("Unable to HUP tunables: {:?}", err)
                        }
                    }
                    _ => warn!("No handler for signal: {sig}"),
                }
            }
        })?;
    let _ = handle.join();
    warn!("Main thread exiting");
    Ok(())
}

#[cfg(feature = "flamegraphs")]
fn memory_debug() {
    // To use this, install "inferno" with `cargo install inferno`.
    // When you want to make the flamegraph, run:
    // inferno-flamegraph /tmp/lqosd-mem.svg > output.svg
    // You can then view output.svg in a web browser.
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let mut fb = allocative::FlameGraphBuilder::default();
            fb.visit_global_roots();
            // fb.visit_root(&*THROUGHPUT_TRACKER);
            // lqos_network_devices::with_network_json_read(|net_json| fb.visit_root(net_json));
            let flamegraph_src = fb.finish();
            let flamegraph_src = flamegraph_src.flamegraph();
            let Ok(mut file) = std::fs::File::create("/tmp/lqosd-mem.svg") else {
                error!("Unable to write flamegraph.");
                continue;
            };
            let _ = file.write_all(flamegraph_src.write().as_bytes());
            info!("Wrote flamegraph to /tmp/lqosd-mem.svg");
        }
    });
}

#[cfg(not(feature = "flamegraphs"))]
fn memory_debug() {}

fn update_lqosd_config_from_bus(config: &lqos_config::Config) -> BusResponse {
    match lqos_config::update_config(config) {
        Ok(()) => {
            BusResponse::Ack
        }
        Err(err) => {
            error!("Error updating config: {err:?}");
            BusResponse::Fail(err.to_string())
        }
    }
}

fn handle_bus_requests(requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
    for req in requests.iter() {
        //println!("Request: {:?}", req);
        BUS_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        responses.push(match req {
            BusRequest::Ping => BusResponse::Ack,
            BusRequest::GetCurrentThroughput => throughput_tracker::current_throughput(),
            BusRequest::GetHostCounter => throughput_tracker::host_counters(),
            BusRequest::GetTopNDownloaders { start, end } => {
                throughput_tracker::top_n(*start, *end)
            }
            BusRequest::GetTopNUploaders { start, end } => {
                throughput_tracker::top_n_up(*start, *end)
            }
            BusRequest::GetCircuitHeatmaps => throughput_tracker::circuit_heatmaps(),
            BusRequest::GetSiteHeatmaps => throughput_tracker::site_heatmaps(),
            BusRequest::GetAsnHeatmaps => throughput_tracker::asn_heatmaps(),
            BusRequest::GetGlobalHeatmap => throughput_tracker::global_heatmap(),
            BusRequest::GetExecutiveSummaryHeader => throughput_tracker::executive_summary_header(),
            BusRequest::GetWorstRtt { start, end } => throughput_tracker::worst_n(*start, *end),
            BusRequest::GetWorstRetransmits { start, end } => {
                throughput_tracker::worst_n_retransmits(*start, *end)
            }
            BusRequest::GetBestRtt { start, end } => throughput_tracker::best_n(*start, *end),
            BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
            BusRequest::RttHistogram => throughput_tracker::rtt_histogram::<50>(),
            BusRequest::ProbeBatch {
                requests,
                max_age_ms,
            } => match probe_provider::probe_batch_blocking(
                requests.clone(),
                Duration::from_millis(*max_age_ms),
            ) {
                Ok(observations) => BusResponse::ProbeObservations(observations),
                Err(err) => BusResponse::Fail(err),
            },
            BusRequest::BridgePublishStatus { generation } => {
                // K1b §4.6 pin 3: the read-only outcome-ring lookup. The
                // bridge broker records what it already logs; this arm never
                // mutates it.
                let status = crate::bridge_broker::outcomes::publish_outcomes()
                    .into_iter()
                    .find(|record| &record.generation == generation)
                    .map(|record| match record.state {
                        crate::bridge_broker::outcomes::PublishState::Pending {
                            seq_pos,
                            seq_len,
                        } => lqos_bus::BridgePublishStatus::Pending { seq_pos, seq_len },
                        crate::bridge_broker::outcomes::PublishState::Retrying {
                            attempt,
                            max,
                        } => lqos_bus::BridgePublishStatus::Retrying { attempt, max },
                        crate::bridge_broker::outcomes::PublishState::Applied => {
                            lqos_bus::BridgePublishStatus::Applied
                        }
                        crate::bridge_broker::outcomes::PublishState::Rejected { reason } => {
                            lqos_bus::BridgePublishStatus::Rejected { reason }
                        }
                    });
                BusResponse::BridgePublishStatus {
                    generation: generation.clone(),
                    status,
                }
            }
            BusRequest::HostCounts => throughput_tracker::host_counts(),
            BusRequest::AllUnknownIps => throughput_tracker::all_unknown_ips(),
            BusRequest::ReloadLibreQoS => program_control::reload_libre_qos(),
            BusRequest::GetRawQueueData(circuit_id) => {
                // `.115`: bridge-analytics snapshot from the in-process
                // last-reply cache. No analytics-socket call on this path; a
                // miss is an explicit None (the API maps it to 200 + null).
                BusResponse::RawQueueData(
                    crate::bridge_analytics::raw_queue_snapshot(&circuit_id).map(Box::new),
                )
            }
            BusRequest::WatchQueue(_circuit_id) => lqos_bus::BusResponse::Ack,
            BusRequest::UpdateLqosDTuning(..) => tuning::tune_lqosd_from_bus(req),
            BusRequest::UpdateLqosdConfig(config) => update_lqosd_config_from_bus(config),
            BusRequest::CreateDynamicCircuit { shaped_device } => {
                crate::dynamic_circuits::create_dynamic_circuit((**shaped_device).clone())
            }
            BusRequest::RemoveDynamicCircuit { circuit_id } => {
                crate::dynamic_circuits::remove_dynamic_circuit(circuit_id)
            }
            BusRequest::InvalidateAuthCache => {
                crate::node_manager::invalidate_auth_cache();
                BusResponse::Ack
            }
            #[cfg(feature = "equinix_tests")]
            BusRequest::RequestLqosEquinixTest => lqos_daht_test::lqos_daht_test(),
            BusRequest::ValidateShapedDevicesCsv => validation::validate_shaped_devices_csv(),
            BusRequest::GetNetworkMap { parent } => {
                shaped_devices_tracker::get_one_network_map_layer(*parent)
            }
            BusRequest::GetFullNetworkMap => shaped_devices_tracker::get_full_network_map(),
            BusRequest::TopMapQueues(n_queues) => {
                shaped_devices_tracker::get_top_n_root_queues(*n_queues)
            }
            BusRequest::GetNodeNamesFromIds(nodes) => shaped_devices_tracker::map_node_names(nodes),
            BusRequest::GetAllCircuits => shaped_devices_tracker::get_all_circuits(),
            BusRequest::GetCircuitById { circuit_id } => {
                shaped_devices_tracker::get_circuit_by_id(circuit_id.clone())
            }
            BusRequest::GetAllCircuitRollups => BusResponse::CircuitRollups(
                shaped_devices_tracker::circuit_live::all_circuit_rollups(),
            ),
            BusRequest::GetCircuitRollupById { circuit_id } => BusResponse::CircuitRollup(
                shaped_devices_tracker::circuit_live::circuit_rollup_by_id(circuit_id),
            ),
            BusRequest::GetFunnel { target: parent } => shaped_devices_tracker::get_funnel(parent),
            BusRequest::GetLqosStats => BusResponse::LqosdStats {
                bus_requests: BUS_REQUESTS.load(std::sync::atomic::Ordering::Relaxed),
                time_to_poll_hosts: TIME_TO_POLL_HOSTS.load(std::sync::atomic::Ordering::Relaxed),
                // K3-B: the kernel-fed high-watermark and RTT-event counters are
                // retired; bridge telemetry reports zeros here.
                high_watermark: lqos_utils::units::DownUpOrder::zeroed(),
                tracked_flows: live_active_flow_count(),
                rtt_events_per_second: 0,
            },
            BusRequest::GetPacketHeaderDump(id) => {
                BusResponse::PacketDump(n_second_packet_dump(*id))
            }
            BusRequest::GetPcapDump(id) => BusResponse::PcapDump(lqos_heimdall::n_second_pcap(*id)),
            BusRequest::GatherPacketData(ip) => {
                let ip = ip.parse::<IpAddr>();
                if let Ok(ip) = ip {
                    if let Some((session_id, countdown)) =
                        crate::heimdall::start_analysis_privileged(ip)
                    {
                        BusResponse::PacketCollectionSession {
                            session_id,
                            countdown,
                        }
                    } else {
                        BusResponse::Fail("Busy".to_string())
                    }
                } else {
                    BusResponse::Fail("Invalid IP".to_string())
                }
            }
            BusRequest::DumpActiveFlows => throughput_tracker::dump_active_flows(),
            BusRequest::CountActiveFlows => throughput_tracker::count_active_flows(),
            BusRequest::TopFlows { n, flow_type } => throughput_tracker::top_flows(*n, *flow_type),
            BusRequest::FlowsByIp(ip) => throughput_tracker::flows_by_ip(ip),
            BusRequest::CurrentEndpointsByCountry => {
                throughput_tracker::current_endpoints_by_country()
            }
            BusRequest::CurrentEndpointLatLon => throughput_tracker::current_lat_lon(),
            BusRequest::EtherProtocolSummary => throughput_tracker::ether_protocol_summary(),
            BusRequest::IpProtocolSummary => throughput_tracker::ip_protocol_summary(),
            BusRequest::FlowDuration => throughput_tracker::flow_duration(),
            BusRequest::BlackboardFinish => {
                if let Some(sender) = BLACKBOARD_SENDER.get() {
                    let _ = sender.send(BlackboardCommand::FinishSession);
                }
                BusResponse::Ack
            }
            BusRequest::BlackboardData {
                subsystem,
                key,
                value,
            } => {
                if let Some(sender) = BLACKBOARD_SENDER.get() {
                    let _ = sender.send(BlackboardCommand::BlackboardData {
                        subsystem: *subsystem,
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                }
                BusResponse::Ack
            }
            BusRequest::BlackboardBlob { tag, part, blob } => {
                if let Some(sender) = BLACKBOARD_SENDER.get() {
                    let _ = sender.send(BlackboardCommand::BlackboardBlob {
                        tag: tag.to_string(),
                        part: *part,
                        blob: blob.clone(),
                    });
                }
                BusResponse::Ack
            }
            BusRequest::GetStormguardStats => {
                let cloned = {
                    let lock = STORMGUARD_STATS.lock();
                    (*lock).clone()
                };
                BusResponse::StormguardStats(cloned)
            }
            BusRequest::GetStormguardDebug => {
                let cloned = {
                    let lock = STORMGUARD_DEBUG.lock();
                    (*lock).clone()
                };
                BusResponse::StormguardDebug(cloned)
            }
            BusRequest::GetStormguardRuntimeStatus => {
                BusResponse::StormguardRuntimeStatus(lqos_stormguard::runtime_status())
            }
            BusRequest::ApiReady => {
                tool_status::api_seen();
                BusResponse::Ack
            }
            BusRequest::ChatbotReady => {
                tool_status::chatbot_seen();
                BusResponse::Ack
            }
            BusRequest::SchedulerReady => {
                tool_status::scheduler_seen();
                BusResponse::Ack
            }
            BusRequest::SchedulerError(error) => {
                tool_status::scheduler_seen();
                tool_status::scheduler_error(Some(error.clone()));
                BusResponse::Ack
            }
            BusRequest::SchedulerOutput(output) => {
                tool_status::scheduler_seen();
                tool_status::scheduler_output(Some(output.clone()));
                BusResponse::Ack
            }
            BusRequest::SchedulerProgress(progress) => {
                tool_status::scheduler_seen();
                let mut progress = progress.clone();
                if progress.updated_unix.is_none() {
                    progress.updated_unix = unix_now().ok();
                }
                tool_status::scheduler_progress(Some(progress));
                BusResponse::Ack
            }
            BusRequest::SchedulerIntegrationStatus(status) => {
                tool_status::scheduler_seen();
                tool_status::scheduler_integration_status(Some(status.clone()));
                BusResponse::Ack
            }
            // AVAIL 0(c) (CARD §2): the read-only convergence-record consumer.
            // The broker is the sole writer; this only clones the published
            // snapshot (never blocks on the broker actor).
            BusRequest::GetConvergenceState => {
                // WP5 (`docs/wp5_seam_state_surface.md`): the SEAM_STATE
                // reconciliation snapshot (replaces the retired AVAIL 0(c)
                // convergence record).
                let snapshot = crate::bridge_broker::convergence_snapshot();
                BusResponse::ConvergenceState(
                    serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string()),
                )
            }
            // `lqos-74t.118.2`: the read-only CPU & Workers snapshot. The
            // broker is the sole writer; this clones the published document and
            // merges the per-request bridge/autobalance view (never blocks the
            // broker actor).
            BusRequest::GetCpuWorkers => {
                BusResponse::CpuWorkers(crate::bridge_broker::cpu_workers_json())
            }
            BusRequest::LogInfo(msg) => {
                info!("BUS LOG: {}", msg);
                BusResponse::Ack
            }
            BusRequest::CheckSchedulerStatus => {
                let running = tool_status::is_scheduler_available();
                let error = tool_status::scheduler_error_message();
                let progress = tool_status::scheduler_progress_state();
                BusResponse::SchedulerStatus {
                    running,
                    error,
                    progress,
                }
            }
            BusRequest::SubmitUrgentIssue { source, severity, code, message, context, dedupe_key } => {
                urgent::submit(*source, *severity, code.clone(), message.clone(), context.clone(), dedupe_key.clone());
                BusResponse::Ack
            }
            BusRequest::GetUrgentIssues => {
                let list = urgent::list();
                BusResponse::UrgentIssues(list)
            }
            BusRequest::ClearUrgentIssue(id) => {
                urgent::clear(*id);
                BusResponse::Ack
            }
            BusRequest::ClearUrgentIssueByIdentity { code, dedupe_key } => {
                urgent::clear_by_identity(code, dedupe_key);
                BusResponse::Ack
            }
            BusRequest::ClearAllUrgentIssues => {
                urgent::clear_all();
                BusResponse::Ack
            }
            BusRequest::GetGlobalWarnings => {
                let warnings = node_manager::get_global_warnings()
                    .into_iter()
                    .map(|(level, message)| (map_warning_level(level), message))
                    .collect();
                BusResponse::GlobalWarnings(warnings)
            }
            BusRequest::GetDeviceCounts => {
                let data = node_manager::device_count();
                BusResponse::DeviceCounts(lqos_bus::DeviceCounts {
                    shaped_devices: data.shaped_devices,
                    unknown_ips: data.unknown_ips,
                })
            }
            BusRequest::GetCircuitCount => {
                let data = node_manager::circuit_count_data();
                BusResponse::CircuitCount(lqos_bus::CircuitCount {
                    count: data.count,
                    configured_count: data.configured_count,
                })
            }
            BusRequest::GetFlowMap => {
                let points = node_manager::flow_map_data()
                    .into_iter()
                    .map(
                        |(lat, lon, country, bytes_sent, rtt_nanos, _bytes_up)| {
                            lqos_bus::FlowMapPoint {
                                lat,
                                lon,
                                country,
                                bytes_sent,
                                rtt_nanos,
                            }
                        },
                    )
                    .collect();
                BusResponse::FlowMap(points)
            }
            BusRequest::GetAsnList => {
                let entries = node_manager::asn_list_data()
                    .into_iter()
                    .map(|entry| lqos_bus::AsnListEntry {
                        count: entry.count,
                        asn: entry.asn,
                        name: entry.name,
                    })
                    .collect();
                BusResponse::AsnList(entries)
            }
            BusRequest::GetCountryList => {
                let entries = node_manager::country_list_data()
                    .into_iter()
                    .map(|entry| lqos_bus::CountryListEntry {
                        count: entry.count,
                        name: entry.name,
                        iso_code: entry.iso_code,
                    })
                    .collect();
                BusResponse::CountryList(entries)
            }
            BusRequest::GetProtocolList => {
                let entries = node_manager::protocol_list_data()
                    .into_iter()
                    .map(|entry| lqos_bus::ProtocolListEntry {
                        count: entry.count,
                        protocol: entry.protocol,
                    })
                    .collect();
                BusResponse::ProtocolList(entries)
            }
            BusRequest::GetAsnFlowTimeline { asn } => {
                let entries = node_manager::flow_timeline_data(*asn)
                    .unwrap_or_else(|err| {
                        warn!("Unable to build ASN flow timeline: {err}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                let buckets = node_manager::asn_timeline_buckets(*asn)
                    .into_iter()
                    .map(flow_bucket_to_bus)
                    .collect();
                BusResponse::AsnFlowTimeline { entries, buckets }
            }
            BusRequest::GetCountryFlowTimeline { iso_code } => {
                let entries = node_manager::country_timeline_data(iso_code)
                    .unwrap_or_else(|err| {
                        warn!("Unable to build country flow timeline: {err}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                let buckets = node_manager::country_timeline_buckets(iso_code)
                    .into_iter()
                    .map(flow_bucket_to_bus)
                    .collect();
                BusResponse::CountryFlowTimeline { entries, buckets }
            }
            BusRequest::GetProtocolFlowTimeline { protocol } => {
                let entries = node_manager::protocol_timeline_data(protocol)
                    .unwrap_or_else(|err| {
                        warn!("Unable to build protocol flow timeline: {err}");
                        Vec::new()
                    })
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                let buckets = node_manager::protocol_timeline_buckets(
                    &protocol.replace("_", "/"),
                )
                .into_iter()
                .map(flow_bucket_to_bus)
                .collect();
                BusResponse::ProtocolFlowTimeline { entries, buckets }
            }
            BusRequest::GetSchedulerDetails => {
                let details = node_manager::scheduler_details_data();
                BusResponse::SchedulerDetails(lqos_bus::SchedulerDetails {
                    available: details.available,
                    error: details.error,
                    details: details.details,
                })
            }
            BusRequest::GetQueueStatsTotal => {
                let totals = queue_stats_total_data();
                BusResponse::QueueStatsTotal(totals)
            }
            BusRequest::GetQoo => {
                let data = node_manager::local_api::executive::qoo_global();
                BusResponse::Qoo(Some(data))
            }
            BusRequest::GetSiteQoo { site_name } => {
                let data = node_manager::local_api::executive::qoo_site(site_name);
                BusResponse::Qoo(data)
            }
            BusRequest::GetCircuitQoo { circuit_id } => {
                let data = node_manager::local_api::executive::qoo_circuit(circuit_id);
                BusResponse::Qoo(data)
            }
            BusRequest::GetCircuitCapacity => {
                let data = circuit_capacity_data();
                BusResponse::CircuitCapacity(data)
            }
            BusRequest::GetTreeCapacity => {
                let data = tree_capacity_data();
                BusResponse::TreeCapacity(data)
            }
            BusRequest::GetRetransmitSummary => {
                let data = retransmit_summary_data();
                BusResponse::RetransmitSummary(data)
            }
            BusRequest::GetTreeSummaryL2 => {
                let data = tree_summary_l2_data();
                BusResponse::TreeSummaryL2(data)
            }
            BusRequest::Search { term } => {
                let results =
                    node_manager::search_results(node_manager::SearchRequest { term: term.clone() })
                .into_iter()
                .map(search_result_to_bus)
                .collect();
                BusResponse::SearchResults(results)
            }
            BusRequest::GetLtsCapabilities => {
                BusResponse::LtsCapabilitiesSummary(crate::lts2_sys::current_capabilities())
            }
            BusRequest::GetInsightLicenseSummary => {
                let (licensed, max_circuits) =
                    crate::lts2_sys::license_grant::current_license_limits();
                BusResponse::InsightLicenseSummary(InsightLicenseSummary {
                    licensed,
                    max_circuits,
                })
            }
            BusRequest::UpdateLqosdConfigPreserveApiCredentials(config) => {
                let _guard =
                    node_manager::local_api::local_api_keys::lock_config_update_blocking();
                match lqos_config::load_config() {
                    Ok(current) => {
                        let config = node_manager::local_api::local_api_keys::preserve_api_credentials(
                            (**config).clone(),
                            &current,
                        );
                        update_lqosd_config_from_bus(&config)
                    }
                    Err(error) => {
                        error!("Unable to load config while preserving local API keys: {error:?}");
                        BusResponse::Fail(error.to_string())
                    }
                }
            }
        });
    }
}

fn map_warning_level(level: node_manager::WarningLevel) -> lqos_bus::WarningLevel {
    match level {
        node_manager::WarningLevel::Info => lqos_bus::WarningLevel::Info,
        node_manager::WarningLevel::Warning => lqos_bus::WarningLevel::Warning,
        node_manager::WarningLevel::Error => lqos_bus::WarningLevel::Error,
    }
}

fn flow_timeline_to_bus(entry: node_manager::FlowTimeline) -> lqos_bus::FlowTimelineEntry {
    lqos_bus::FlowTimelineEntry {
        start: entry.start,
        end: entry.end,
        duration_nanos: entry.duration_nanos,
        throughput: entry.throughput,
        tcp_retransmits: entry.tcp_retransmits,
        rtt_nanos: [entry.rtt[0].as_nanos(), entry.rtt[1].as_nanos()],
        retransmit_times_down: entry.retransmit_times_down,
        retransmit_times_up: entry.retransmit_times_up,
        total_bytes: entry.total_bytes,
        protocol: entry.protocol,
        circuit_id: entry.circuit_id,
        circuit_name: entry.circuit_name,
        remote_ip: entry.remote_ip,
    }
}

fn flow_bucket_to_bus(
    bucket: node_manager::FlowTimelineBucket,
) -> lqos_bus::FlowTimelineBucket {
    lqos_bus::FlowTimelineBucket {
        start_millis: bucket.start_millis,
        sampled_intervals: bucket.sampled_intervals,
        bytes: bucket.bytes,
        flows: bucket.flows,
    }
}

fn queue_stats_total_data() -> lqos_bus::QueueStatsTotal {
    lqos_bus::QueueStatsTotal {
        marks: lqos_utils::units::DownUpOrder::new(
            lqos_queue_tracker::TOTAL_QUEUE_STATS.marks.get_down(),
            lqos_queue_tracker::TOTAL_QUEUE_STATS.marks.get_up(),
        ),
        drops: lqos_utils::units::DownUpOrder::new(
            lqos_queue_tracker::TOTAL_QUEUE_STATS.drops.get_down(),
            lqos_queue_tracker::TOTAL_QUEUE_STATS.drops.get_up(),
        ),
    }
}

fn circuit_capacity_data() -> Vec<lqos_bus::CircuitCapacityRow> {
    use crate::throughput_tracker::THROUGHPUT_TRACKER;
    use lqos_utils::units::DownUpOrder;

    // K2 slice-3 3a: on a bridge box the per-circuit interval bytes + RTT p50
    // are in the actor cache; the kernel tracker is never written post-K3.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let catalog = lqos_network_devices::shaped_devices_catalog();
        return snapshot
            .by_circuit_id
            .iter()
            .filter_map(|(circuit_id, row)| {
                catalog
                    .iter_devices()
                    .find(|device| device.circuit_id == *circuit_id)
                    .map(|device| {
 // 3b demand-source sweep: capacity headroom
                        // is read against the DELIVERED (shaped) rate.
                        let down_mbps =
                            (row.shaped_bytes_per_second.down as f64 * 8.0) / 1_000_000.0;
                        let down = down_mbps / device.download_max_mbps as f64;
                        let up_mbps =
                            (row.shaped_bytes_per_second.up as f64 * 8.0) / 1_000_000.0;
                        let up = up_mbps / device.upload_max_mbps as f64;
                        lqos_bus::CircuitCapacityRow {
                            circuit_name: device.circuit_name.clone(),
                            circuit_id: circuit_id.clone(),
                            capacity: [down, up],
                            median_rtt: row
                                .rtt_current_p50_nanos
                                .down
                                .map(|nanos| nanos as f32 / 1_000_000.0)
                                .unwrap_or(0.0),
                        }
                    })
            })
            .collect();
    }

    use std::collections::HashMap;

    struct CircuitAccumulator {
        bytes: DownUpOrder<u64>,
        median_rtt: f32,
    }

    let mut circuits: HashMap<String, CircuitAccumulator> = HashMap::new();

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .for_each(|(_k, c)| {
            if let Some(circuit_id) = &c.circuit_id {
                if let Some(accumulator) = circuits.get_mut(circuit_id) {
                    accumulator.bytes += c.bytes_per_second;
                    if let Some(latency) = c.median_latency() {
                        accumulator.median_rtt = latency;
                    }
                } else {
                    circuits.insert(
                        circuit_id.clone(),
                        CircuitAccumulator {
                            bytes: c.bytes_per_second,
                            median_rtt: c.median_latency().unwrap_or(0.0),
                        },
                    );
                }
            }
        });

    let catalog = lqos_network_devices::shaped_devices_catalog();
    circuits
        .iter()
        .filter_map(|(circuit_id, accumulator)| {
            catalog
                .iter_devices()
                .find(|sd| sd.circuit_id == *circuit_id)
                .map(|device| {
                    let down_mbps = (accumulator.bytes.down as f64 * 8.0) / 1_000_000.0;
                    let down = down_mbps / device.download_max_mbps as f64;
                    let up_mbps = (accumulator.bytes.up as f64 * 8.0) / 1_000_000.0;
                    let up = up_mbps / device.upload_max_mbps as f64;

                    lqos_bus::CircuitCapacityRow {
                        circuit_name: device.circuit_name.clone(),
                        circuit_id: circuit_id.clone(),
                        capacity: [down, up],
                        median_rtt: accumulator.median_rtt,
                    }
                })
        })
        .collect()
}

fn tree_capacity_data() -> Vec<lqos_bus::NodeCapacity> {
    lqos_network_devices::with_network_json_read(|net_json| {
        net_json
            .get_nodes_when_ready()
            .iter()
            .enumerate()
            .map(|(id, node)| {
                let mut node = crate::shaped_devices_tracker::node_to_transport(node);
                // Cluster 5 (kernel_gating_audit_card §2.5): the bus arm
                // overlays like the TreeCapacity ticker.
                crate::bridge_analytics::overlay_live_fields(id, &mut node);
                let down = node.current_throughput.0 as f64 * 8.0 / 1_000_000.0;
                let up = node.current_throughput.1 as f64 * 8.0 / 1_000_000.0;
                let effective_max = node.effective_max_throughput.unwrap_or(node.max_throughput);
                let max_down = effective_max.0;
                let max_up = effective_max.1;
                let median_rtt = if node.rtts.is_empty() {
                    0.0
                } else {
                    let n = node.rtts.len() / 2;
                    if node.rtts.len().is_multiple_of(2) {
                        (node.rtts[n - 1] + node.rtts[n]) / 2.0
                    } else {
                        node.rtts[n]
                    }
                };

                lqos_bus::NodeCapacity {
                    id,
                    name: node.name.clone(),
                    down,
                    up,
                    max_down,
                    max_up,
                    median_rtt,
                }
            })
            .collect()
    })
}

fn retransmit_summary_data() -> lqos_bus::RetransmitSummary {
    let data = crate::throughput_tracker::min_max_median_tcp_retransmits();
    lqos_bus::RetransmitSummary {
        up: data.up,
        down: data.down,
        tcp_up: data.tcp_up,
        tcp_down: data.tcp_down,
    }
}

fn tree_summary_l2_data() -> Vec<(usize, Vec<(usize, lqos_config::NetworkJsonTransport)>)> {
    use std::collections::BTreeMap;

    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let mut candidates: Vec<(usize, usize, lqos_config::NetworkJsonTransport, u64)> =
            Vec::new();

        for (p_idx, p_node) in nodes.iter().enumerate() {
            if p_node.immediate_parent == Some(0) {
                for (c_idx, c_node) in nodes.iter().enumerate() {
                    if c_node.immediate_parent == Some(p_idx) {
                        let mut t = crate::shaped_devices_tracker::node_to_transport(c_node);
                        // Cluster 5 (kernel_gating_audit_card §2.5): the bus
                        // arm overlays like the TreeSummaryL2 ticker (so the
                        // top-N is ranked on live throughput).
                        crate::bridge_analytics::overlay_live_fields(c_idx, &mut t);
                        let total = t.current_throughput.0 + t.current_throughput.1;
                        candidates.push((p_idx, c_idx, t, total));
                    }
                }
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.3));
        let n: usize = 10;
        if candidates.len() > n {
            candidates.truncate(n);
        }

        let mut map: BTreeMap<usize, Vec<(usize, lqos_config::NetworkJsonTransport)>> =
            BTreeMap::new();
        for (p_idx, c_idx, t, _total) in candidates.into_iter() {
            map.entry(p_idx).or_default().push((c_idx, t));
        }

        map.into_iter().collect()
    })
}

fn search_result_to_bus(entry: node_manager::SearchResult) -> lqos_bus::SearchResultEntry {
    match entry {
        node_manager::SearchResult::Circuit { id, name } => {
            lqos_bus::SearchResultEntry::Circuit { id, name }
        }
        node_manager::SearchResult::Device {
            circuit_id,
            name,
            circuit_name,
        } => lqos_bus::SearchResultEntry::Device {
            circuit_id,
            name,
            circuit_name,
        },
        node_manager::SearchResult::Site { idx, name } => {
            lqos_bus::SearchResultEntry::Site { idx, name }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_bus::{UrgentSeverity, UrgentSource};

    #[test]
    fn bus_clear_urgent_issue_by_identity_reaches_urgent_store() {
        let code = "TEST_BUS_XDP_IP_MAPPING_APPLY_FAILED";
        urgent::clear_by_identity(code, code);
        urgent::submit(
            UrgentSource::LibreQoS,
            UrgentSeverity::Error,
            code.to_string(),
            "mapping failed".to_string(),
            None,
            Some(code.to_string()),
        );

        let requests = [BusRequest::ClearUrgentIssueByIdentity {
            code: code.to_string(),
            dedupe_key: code.to_string(),
        }];
        let mut responses = Vec::new();
        handle_bus_requests(&requests, &mut responses);

        assert!(matches!(responses.as_slice(), [BusResponse::Ack]));
        assert!(!urgent::list().iter().any(|issue| issue.code == code));
    }

    #[test]
    fn bus_qoo_requests_dispatch_to_qoo_responses() {
        let requests = [
            BusRequest::GetQoo,
            BusRequest::GetSiteQoo {
                site_name: "missing-site".to_string(),
            },
            BusRequest::GetCircuitQoo {
                circuit_id: "missing-circuit".to_string(),
            },
        ];
        let mut responses = Vec::new();

        handle_bus_requests(&requests, &mut responses);

        assert!(matches!(responses.as_slice(), [
            BusResponse::Qoo(Some(global)),
            BusResponse::Qoo(None),
            BusResponse::Qoo(None),
        ] if global.key == "global"));
    }

    /// `.35` failure-surfacing oracle: a wired subsystem failure must reach the
    /// API OUTPUT (the bus response the websocket relays), not merely the sink.
    #[test]
    fn wired_failure_surfaces_in_global_warnings_response() {
        let _guard = test_support::failure_surfacing_test_lock()
            .lock()
            .expect("failure-surfacing test lock should not be poisoned");
        node_manager::clear_global_warnings();
        shaping_runtime::mark_starting("oracle reset");

        let summary = "StormGuard failed to start (oracle)";
        // One composite call is all a caller does; the three writers live here.
        record_shaping_failure(
            shaping_runtime::ShapingRuntimeState::Degraded,
            summary,
            "no probe client",
        );

        // Sink end: the operator warning is recorded at Error level.
        let sink = node_manager::get_global_warnings();
        assert!(
            sink.iter()
                .any(|(level, message)| *level == node_manager::WarningLevel::Error
                    && message.as_str() == summary),
            "the wired failure must populate the warning sink: {sink:?}"
        );

        // Runtime end: the status latched the failure.
        let status = shaping_runtime::get_status();
        assert_eq!(status.state, shaping_runtime::ShapingRuntimeState::Degraded);
        assert!(status.degraded);

        // API end: the bus response that backs the UI carries the warning.
        let requests = [BusRequest::GetGlobalWarnings];
        let mut responses = Vec::new();
        handle_bus_requests(&requests, &mut responses);
        let [BusResponse::GlobalWarnings(list)] = responses.as_slice() else {
            panic!("GetGlobalWarnings must answer with GlobalWarnings: {responses:?}");
        };
        assert!(
            list.iter().any(|(level, message)| {
                *level == lqos_bus::WarningLevel::Error && message.as_str() == summary
            }),
            "the API output must carry the wired failure: {list:?}"
        );

        // The UI switches on the serialized level name; the error must survive
        // the wire as "Error".
        let json = serde_json::to_string(&responses).expect("bus response must serialize");
        assert!(
            json.contains("\"Error\"") && json.contains(summary),
            "serialized API output must expose the Error level: {json}"
        );

        shape_cleanup();
    }

    /// Resets the process-global failure-surfacing state used by these tests.
    fn shape_cleanup() {
        node_manager::clear_global_warnings();
        shaping_runtime::mark_starting("oracle cleanup");
    }

    /// `lqos-74t.2.1.1`: the stock default keeps WARN but lifts the LTS-volume
    /// diagnostic target to INFO — the one home for the directive string. The
    /// E2E oracle proves the journal effect on a stock `.12`.
    #[test]
    fn default_filter_lifts_the_lts_volume_target() {
        assert_eq!(default_filter_directives(), "warn,lqos_lts_volume=info");
    }

    /// `lqos-74t.2.1.1`: EVERY LTS-volume instrument carries the dedicated
    /// target, so the stock filter actually lifts it (a removed tag silently
    /// re-hides the instrument — the original bug).
    #[test]
    fn lts_volume_instruments_carry_the_dedicated_target() {
        const T: &str = "target: \"lqos_lts_volume\"";
        let mq = include_str!("lts2_sys/lts2_client/ingestor/message_queue.rs");
        let ing = include_str!("lts2_sys/lts2_client/ingestor.rs");
        let ba = include_str!("bridge_analytics/mod.rs");
        assert_eq!(mq.matches(T).count(), 4, "message_queue.rs");
        assert_eq!(ing.matches(T).count(), 1, "ingestor.rs");
        assert_eq!(ba.matches(T).count(), 1, "bridge_analytics/mod.rs");
    }
}
