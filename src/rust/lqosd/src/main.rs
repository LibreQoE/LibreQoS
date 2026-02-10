//! `lqosd` is the core of LibreQoS. It runs as a daemon, loads the XDP,
//! manages TC creation, and provides the web interface.

#![deny(clippy::unwrap_used)]

mod blackboard;
mod file_lock;
mod ip_mapping;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
pub mod lts2_sys;
mod node_manager;
mod preflight_checks;
mod program_control;
mod remote_commands;
mod scheduler_control;
mod shaped_devices_tracker;
mod stats;
mod stick;
mod system_stats;
mod throughput_tracker;
mod tool_status;
mod tuning;
mod urgent;
mod validation;
mod version_checks;

#[cfg(feature = "flamegraphs")]
use std::io::Write;
use std::net::IpAddr;

use crate::ip_mapping::clear_hot_cache;
use crate::{
    file_lock::FileLock,
    ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow},
    throughput_tracker::flow_data::{FlowActor, flowbee_handle_events, setup_netflow_tracker},
};
#[cfg(feature = "flamegraphs")]
use allocative::Allocative;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse, UnixSocketServer};
use lqos_heimdall::{n_second_packet_dump, perf_interface::heimdall_handle_events, start_heimdall};
use lqos_queue_tracker::{
    add_watched_queue, get_raw_circuit_data, spawn_queue_monitor, spawn_queue_structure_monitor,
};
use lqos_sys::LibreQoSKernels;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use stats::{BUS_REQUESTS, FLOWS_TRACKED, HIGH_WATERMARK, TIME_TO_POLL_HOSTS};
use std::{thread, time::Duration};
use throughput_tracker::flow_data::get_rtt_events_per_second;
use tracing::{error, info, warn};

// Use MiMalloc only on supported platforms
//#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
//use mimalloc::MiMalloc;

use crate::blackboard::{BLACKBOARD_SENDER, BlackboardCommand};

use crate::lts2_sys::get_lts_license_status;
use crate::lts2_sys::shared_types::LtsStatus;
use crate::remote_commands::start_remote_commands;
#[cfg(feature = "flamegraphs")]
use crate::shaped_devices_tracker::NETWORK_JSON;
#[cfg(feature = "flamegraphs")]
use crate::throughput_tracker::THROUGHPUT_TRACKER;
#[cfg(feature = "flamegraphs")]
use crate::throughput_tracker::flow_data::{ALL_FLOWS, RECENT_FLOWS};
use lqos_stormguard::{STORMGUARD_DEBUG, STORMGUARD_STATS};
use tracing::level_filters::LevelFilter;
// Use MiMalloc only on supported platforms
// #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
// #[global_allocator]
// static GLOBAL: MiMalloc = MiMalloc;

/// Configure a highly detailed logging system.
pub fn set_console_logging() -> anyhow::Result<()> {
    // install global collector configured based on RUST_LOG env var.
    let level = if let Ok(level) = std::env::var("RUST_LOG") {
        match level.to_lowercase().as_str() {
            "trace" => LevelFilter::TRACE,
            "debug" => LevelFilter::DEBUG,
            "info" => LevelFilter::INFO,
            "warn" => LevelFilter::WARN,
            "error" => LevelFilter::ERROR,
            _ => LevelFilter::WARN,
        }
    } else {
        LevelFilter::WARN
    };

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        // Use a more compact, abbreviated log format
        .compact()
        // Display source code file paths
        .with_file(true)
        // Display source code line numbers
        .with_line_number(true)
        // Display the thread ID an event was recorded on
        .with_thread_ids(false)
        // Don't display the event's target (module path)
        .with_target(false)
        // Build the subscriber
        .finish();

    // Set the subscriber as the default
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

fn normalize_mapping_request(
    tc_handle: lqos_bus::TcHandle,
    cpu: u32,
    upload: bool,
) -> anyhow::Result<(lqos_bus::TcHandle, u32, bool)> {
    let config = lqos_config::load_config()?;

    // With derived upload mapping, we only store one mapping in the kernel.
    // In non-stick mode, `upload` is meaningless; treat it as false.
    if !config.on_a_stick_mode() {
        return Ok((tc_handle, cpu, false));
    }

    let stick_offset = stick::stick_offset();
    if stick_offset == 0 {
        return Ok((tc_handle, cpu, false));
    }

    if !upload {
        return Ok((tc_handle, cpu, false));
    }

    let base_cpu = cpu.checked_sub(stick_offset).ok_or_else(|| {
        anyhow::anyhow!(
            "On-a-stick upload mapping CPU ({cpu}) is less than stick_offset ({stick_offset})."
        )
    })?;

    let (major, minor) = tc_handle.get_major_minor();
    let stick_offset_u16 = u16::try_from(stick_offset).map_err(|_| {
        anyhow::anyhow!(
            "stick_offset ({stick_offset}) exceeds u16 range; cannot normalize tc_handle."
        )
    })?;
    let base_major = major.checked_sub(stick_offset_u16).ok_or_else(|| {
        anyhow::anyhow!(
            "On-a-stick upload mapping tc_handle major ({major}) is less than stick_offset ({stick_offset})."
        )
    })?;

    let base_tc_handle = lqos_bus::TcHandle::from_u32(((base_major as u32) << 16) | minor as u32);
    Ok((base_tc_handle, base_cpu, false))
}

fn main() -> Result<()> {
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

    // Run preflight checks
    preflight_checks::preflight_checks()?;

    // Load config
    let config = lqos_config::load_config()?;
    let stick_offset = stick::recompute_stick_offset(&config)?;

    if let Err(e) = lts2_sys::license_grant::init_license_storage(&config) {
        warn!("Failed to initialize Insight license storage: {e:?}");
    }

    // Apply Tunings
    tuning::tune_lqosd_from_config_file()?;

    // Start the flow tracking actor. This has to happen
    // before the ringbuffer goes live.
    FlowActor::start()?;

    // Start the XDP/TC kernels
    let kernels = if config.on_a_stick_mode() {
        LibreQoSKernels::on_a_stick_mode(
            &config.internet_interface(),
            config.stick_vlans().1 as u16,
            config.stick_vlans().0 as u16,
            stick_offset,
            Some(heimdall_handle_events),
            Some(flowbee_handle_events),
        )?
    } else {
        LibreQoSKernels::new(
            &config.internet_interface(),
            &config.isp_interface(),
            Some(heimdall_handle_events),
            Some(flowbee_handle_events),
        )?
    };

    // Start the Bakery for TC command execution
    let Ok(bakery_sender) = lqos_bakery::start_bakery() else {
        error!("Failed to start Bakery, exiting.");
        std::process::exit(1);
    };

    // Spawn tracking sub-systems
    let Ok(control_channel) = lts2_sys::control_channel::init_control_channel() else {
        error!("Failed to initialize Insight control channel, exiting.");
        std::process::exit(1);
    };
    let control_tx_for_lts = control_channel.tx.clone();
    let control_tx_for_web = control_channel.tx.clone();
    if let Err(e) = lts2_sys::start_lts2(control_tx_for_lts) {
        error!("Failed to start Insight: {:?}", e);
    } else {
        info!("Insight client started successfully");
    }
    let _blackboard_tx = blackboard::start_blackboard();
    start_remote_commands();
    let flow_tx = setup_netflow_tracker()?;
    let _ = throughput_tracker::flow_data::setup_flow_analysis();
    start_heimdall()?;
    spawn_queue_structure_monitor()?;
    shaped_devices_tracker::shaped_devices_watcher()?;
    shaped_devices_tracker::network_json_watcher()?;
    let system_usage_tx = system_stats::start_system_stats()?;
    throughput_tracker::spawn_throughput_monitor(
        flow_tx,
        system_usage_tx.clone(),
        bakery_sender.clone(),
    )?;
    spawn_queue_monitor()?;
    lqos_sys::bpf_garbage_collector();
    version_checks::start_version_check()?;

    // Handle signals
    let mut signals = Signals::new([SIGINT, SIGHUP, SIGTERM])?;
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
                        std::mem::drop(kernels);
                        // Give kernel/driver a moment to finalize detach
                        thread::sleep(Duration::from_millis(50));
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

    // Create the socket server
    let server = UnixSocketServer::new().expect("Unable to spawn server");

    // Memory Debugging
    memory_debug();

    let bakery_sender_for_async = bakery_sender.clone();
    let control_tx_for_webserver = control_tx_for_web.clone();
    let handle = std::thread::Builder::new()
        .name("Async Bus/Web".to_string())
        .spawn(move || {
            let Ok(tokio_runtime) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                error!("Unable to start Tokio runtime. Not much is going to work");
                return;
            };
            tokio_runtime.block_on(async {
                // Notify bakery when the bus socket becomes available
                tokio::spawn(async move {
                    use tokio::time::{Duration, sleep};
                    // Wait up to ~5 seconds for the socket to appear
                    for _ in 0..100u32 {
                        if tokio::fs::metadata(lqos_bus::BUS_SOCKET_PATH).await.is_ok() {
                            break;
                        }
                        sleep(Duration::from_millis(50)).await;
                    }
                    if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                        let _ = sender.send(lqos_bakery::BakeryCommands::BusReady);
                    }
                });

                tokio::spawn(async move {
                    match lts2_sys::control_channel::start_control_channel(control_channel).await {
                        Ok(_) => info!("Insight control channel started successfully"),
                        Err(e) => error!("Insight control channel failed to start: {:#}", e),
                    }

                    match lqos_stormguard::start_stormguard(bakery_sender_for_async).await {
                        Ok(_) => info!("StormGuard started successfully"),
                        Err(e) => error!("StormGuard failed to start: {:#}", e),
                    }
                });

                let (bus_tx, bus_rx) = tokio::sync::mpsc::channel::<(
                    tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
                    BusRequest,
                )>(100);

                // Webserver starting point
                let webserver_disabled = config.disable_webserver.unwrap_or(false);
                if !webserver_disabled {
                    let control_tx_for_webserver = control_tx_for_webserver.clone();
                    tokio::spawn(async move {
                        if let Err(e) = node_manager::spawn_webserver(
                            bus_tx,
                            system_usage_tx,
                            control_tx_for_webserver,
                        )
                        .await
                        {
                            error!("Node Manager Failed: {e:?}");
                        }
                    });
                } else {
                    warn!("Webserver disabled by configuration");
                }

                // Main bus listen loop
                if let Err(e) = server.listen(handle_bus_requests, bus_rx).await {
                    error!("Bus stopped: {e:?}");
                }
            });
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
            // fb.visit_root(&*ALL_FLOWS);
            // fb.visit_root(&*RECENT_FLOWS);
            //fb.visit_root(&*NETWORK_JSON);
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
            BusRequest::MapIpToFlow {
                ip_address,
                tc_handle,
                cpu,
                circuit_id,
                device_id,
                upload,
            } => {
                match normalize_mapping_request(*tc_handle, *cpu, *upload) {
                    Ok((tc_handle, cpu, upload)) => {
                        let resp = map_ip_to_flow(
                            ip_address,
                            &tc_handle,
                            cpu,
                            upload,
                            *circuit_id,
                            *device_id,
                        );
                        if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                            let _ = sender.send(lqos_bakery::BakeryCommands::MapIp {
                                ip_address: ip_address.clone(),
                                tc_handle,
                                cpu,
                                upload,
                            });
                        }
                        resp
                    }
                    Err(e) => BusResponse::Fail(e.to_string()),
                }
            }
            BusRequest::ClearHotCache => {
                // Let the bakery finalize staged mapping changes, then clear hot cache.
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let _ = sender.send(lqos_bakery::BakeryCommands::CommitMappings);
                }
                clear_hot_cache()
            }
            BusRequest::DelIpFlow { ip_address, upload: _ } => {
                // With derived upload mapping, both directions share a single mapping entry.
                // Always delete from the base mapping set.
                let resp = del_ip_flow(ip_address, false);
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let _ = sender.send(lqos_bakery::BakeryCommands::DelIp {
                        ip_address: ip_address.clone(),
                        upload: false,
                    });
                }
                resp
            }
            BusRequest::ClearIpFlow => {
                let resp = clear_ip_flows();
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let _ = sender.send(lqos_bakery::BakeryCommands::ClearIpAll);
                }
                resp
            }
            BusRequest::ListIpFlow => list_mapped_ips(),
            BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
            BusRequest::RttHistogram => throughput_tracker::rtt_histogram::<50>(),
            BusRequest::HostCounts => throughput_tracker::host_counts(),
            BusRequest::AllUnknownIps => throughput_tracker::all_unknown_ips(),
            BusRequest::ReloadLibreQoS => program_control::reload_libre_qos(),
            BusRequest::GetRawQueueData(circuit_id) => get_raw_circuit_data(circuit_id),
            BusRequest::WatchQueue(circuit_id) => {
                add_watched_queue(circuit_id);
                lqos_bus::BusResponse::Ack
            }
            BusRequest::UpdateLqosDTuning(..) => tuning::tune_lqosd_from_bus(req),
            BusRequest::UpdateLqosdConfig(config) => {
                let result = lqos_config::update_config(config);
                if result.is_err() {
                    error!("Error updating config: {:?}", result);
                }
                if let Ok(cfg) = lqos_config::load_config() {
                    let _ = stick::recompute_stick_offset(&cfg);
                }
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
            BusRequest::GetFunnel { target: parent } => shaped_devices_tracker::get_funnel(parent),
            BusRequest::GetLqosStats => BusResponse::LqosdStats {
                bus_requests: BUS_REQUESTS.load(std::sync::atomic::Ordering::Relaxed),
                time_to_poll_hosts: TIME_TO_POLL_HOSTS.load(std::sync::atomic::Ordering::Relaxed),
                high_watermark: HIGH_WATERMARK.as_down_up(),
                tracked_flows: FLOWS_TRACKED.load(std::sync::atomic::Ordering::Relaxed),
                rtt_events_per_second: get_rtt_events_per_second(),
            },
            BusRequest::GetPacketHeaderDump(id) => {
                BusResponse::PacketDump(n_second_packet_dump(*id))
            }
            BusRequest::GetPcapDump(id) => BusResponse::PcapDump(lqos_heimdall::n_second_pcap(*id)),
            BusRequest::GatherPacketData(ip) => {
                let ip = ip.parse::<IpAddr>();
                if let Ok(ip) = ip {
                    if let Some((session_id, countdown)) =
                        lqos_heimdall::hyperfocus_on_target(ip.into())
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
                        subsystem: subsystem.clone(),
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
            BusRequest::BakeryStart => {
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let _ = sender.send(lqos_bakery::BakeryCommands::StartBatch);
                    BusResponse::Ack
                } else {
                    BusResponse::Fail("Bakery not initialized".to_string())
                }
            }
            BusRequest::BakeryCommit => {
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let _ = sender.send(lqos_bakery::BakeryCommands::CommitBatch);
                    BusResponse::Ack
                } else {
                    BusResponse::Fail("Bakery not initialized".to_string())
                }
            }
            BusRequest::BakeryChangeSiteSpeedLive { site_hash, download_bandwidth_min, upload_bandwidth_min, download_bandwidth_max, upload_bandwidth_max } => {
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let command = lqos_bakery::BakeryCommands::ChangeSiteSpeedLive {
                        site_hash: *site_hash,
                        download_bandwidth_min: *download_bandwidth_min,
                        upload_bandwidth_min: *upload_bandwidth_min,
                        download_bandwidth_max: *download_bandwidth_max,
                        upload_bandwidth_max: *upload_bandwidth_max,
                    };
                    let _ = sender.send(command);
                }
                BusResponse::Ack
            }
            BusRequest::BakeryMqSetup {
                queues_available,
                stick_offset,
            } => {
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let _ = sender.send(lqos_bakery::BakeryCommands::MqSetup {
                        queues_available: *queues_available,
                        stick_offset: *stick_offset,
                    });
                    BusResponse::Ack
                } else {
                    BusResponse::Fail("Bakery not initialized".to_string())
                }
            }
            BusRequest::BakeryAddSite {
                site_hash,
                parent_class_id,
                up_parent_class_id,
                class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => {
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let _ = sender.send(lqos_bakery::BakeryCommands::AddSite {
                        site_hash: *site_hash,
                        parent_class_id: parent_class_id.clone(),
                        up_parent_class_id: up_parent_class_id.clone(),
                        class_minor: class_minor.clone(),
                        download_bandwidth_min: *download_bandwidth_min,
                        upload_bandwidth_min: *upload_bandwidth_min,
                        download_bandwidth_max: *download_bandwidth_max,
                        upload_bandwidth_max: *upload_bandwidth_max,
                    });
                    BusResponse::Ack
                } else {
                    BusResponse::Fail("Bakery not initialized".to_string())
                }
            }
            BusRequest::BakeryAddCircuit {
                circuit_hash,
                parent_class_id,
                up_parent_class_id,
                class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
                class_major,
                up_class_major,
                ip_addresses,
                sqm_override,
            } => {
                if let Some(s) = sqm_override.as_ref() {
                    if s.eq_ignore_ascii_case("fq_codel") {
                        tracing::info!(
                            "lqosd: Received BakeryAddCircuit with fq_codel override for circuit_hash={} (parent_class_id={}, up_parent_class_id={}, class_minor=0x{:x})",
                            circuit_hash,
                            parent_class_id.as_tc_string(),
                            up_parent_class_id.as_tc_string(),
                            class_minor
                        );
                    }
                }
                if let Some(sender) = lqos_bakery::BAKERY_SENDER.get() {
                    let sender = sender.clone();
                    let _ = sender.send(lqos_bakery::BakeryCommands::AddCircuit {
                        circuit_hash: *circuit_hash,
                        parent_class_id: *parent_class_id,
                        up_parent_class_id: *up_parent_class_id,
                        class_minor: *class_minor,
                        download_bandwidth_min: download_bandwidth_min.clone(),
                        upload_bandwidth_min: upload_bandwidth_min.clone(),
                        download_bandwidth_max: download_bandwidth_max.clone(),
                        upload_bandwidth_max: upload_bandwidth_max.clone(),
                        class_major: class_major.clone(),
                        up_class_major: up_class_major.clone(),
                        ip_addresses: ip_addresses.clone(),
                        sqm_override: sqm_override.clone(),
                    });
                    BusResponse::Ack
                } else {
                    BusResponse::Fail("Bakery not initialized".to_string())
                }
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
            BusRequest::GetBakeryStats => BusResponse::BakeryActiveCircuits(
                lqos_bakery::ACTIVE_CIRCUITS.load(std::sync::atomic::Ordering::Relaxed),
            ),
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
                tool_status::scheduler_error(Some(error.clone()));
                BusResponse::Ack
            }
            BusRequest::LogInfo(msg) => {
                info!("BUS LOG: {}", msg);
                BusResponse::Ack
            }
            BusRequest::CheckSchedulerStatus => {
                let running = tool_status::is_scheduler_available();
                let error = tool_status::scheduler_error_message();
                BusResponse::SchedulerStatus { running, error }
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
                    .map(|(lat, lon, country, bytes_sent, rtt_nanos)| lqos_bus::FlowMapPoint {
                        lat,
                        lon,
                        country,
                        bytes_sent,
                        rtt_nanos,
                    })
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
                let data = node_manager::flow_timeline_data(*asn)
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                BusResponse::AsnFlowTimeline(data)
            }
            BusRequest::GetCountryFlowTimeline { iso_code } => {
                let data = node_manager::country_timeline_data(iso_code)
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                BusResponse::CountryFlowTimeline(data)
            }
            BusRequest::GetProtocolFlowTimeline { protocol } => {
                let data = node_manager::protocol_timeline_data(protocol)
                    .into_iter()
                    .map(flow_timeline_to_bus)
                    .collect();
                BusResponse::ProtocolFlowTimeline(data)
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
            BusRequest::CheckInsight => {
                let (status, _) = get_lts_license_status();
                match status {
                    LtsStatus::Invalid | LtsStatus::NotChecked => BusResponse::InsightStatus(false),
                    _ => BusResponse::InsightStatus(true)
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
    use crate::shaped_devices_tracker::SHAPED_DEVICES;
    use crate::throughput_tracker::THROUGHPUT_TRACKER;
    use lqos_utils::units::DownUpOrder;
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

    let shaped_devices = SHAPED_DEVICES.load();
    circuits
        .iter()
        .filter_map(|(circuit_id, accumulator)| {
            shaped_devices
                .devices
                .iter()
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
    use crate::shaped_devices_tracker::NETWORK_JSON;

    let net_json = NETWORK_JSON.read();
    net_json
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .map(|(id, node)| {
            let node = node.clone_to_transit();
            let down = node.current_throughput.0 as f64 * 8.0 / 1_000_000.0;
            let up = node.current_throughput.1 as f64 * 8.0 / 1_000_000.0;
            let max_down = node.max_throughput.0 as f64;
            let max_up = node.max_throughput.1 as f64;
            let median_rtt = if node.rtts.is_empty() {
                0.0
            } else {
                let n = node.rtts.len() / 2;
                if node.rtts.len() % 2 == 0 {
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
    use crate::shaped_devices_tracker::NETWORK_JSON;
    use std::collections::BTreeMap;

    let net_json = NETWORK_JSON.read();
    let nodes = net_json.get_nodes_when_ready();
    let mut candidates: Vec<(usize, usize, lqos_config::NetworkJsonTransport, u64)> = Vec::new();

    for (p_idx, p_node) in nodes.iter().enumerate() {
        if p_node.immediate_parent == Some(0) {
            for (c_idx, c_node) in nodes.iter().enumerate() {
                if c_node.immediate_parent == Some(p_idx) {
                    let t = c_node.clone_to_transit();
                    let total = t.current_throughput.0 + t.current_throughput.1;
                    candidates.push((p_idx, c_idx, t, total));
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.3.cmp(&a.3));
    let n: usize = 10;
    if candidates.len() > n {
        candidates.truncate(n);
    }

    let mut map: BTreeMap<usize, Vec<(usize, lqos_config::NetworkJsonTransport)>> = BTreeMap::new();
    for (p_idx, c_idx, t, _total) in candidates.into_iter() {
        map.entry(p_idx).or_default().push((c_idx, t));
    }

    map.into_iter().collect()
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
