mod anonymous_usage;
mod blackboard;
mod file_lock;
mod ip_mapping;
mod long_term_stats;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
pub mod lts2_sys;
mod node_manager;
mod preflight_checks;
mod program_control;
mod remote_commands;
mod shaped_devices_tracker;
mod stats;
mod system_stats;
mod throughput_tracker;
mod tuning;
mod validation;
mod version_checks;
mod scheduler_control;

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
use lqos_bus::{BusRequest, BusResponse, StatsRequest, UnixSocketServer};
use lqos_heimdall::{n_second_packet_dump, perf_interface::heimdall_handle_events, start_heimdall};
use lqos_queue_tracker::{
    add_watched_queue, get_raw_circuit_data, spawn_queue_monitor, spawn_queue_structure_monitor,
};
use lqos_sys::LibreQoSKernels;
use lts_client::collector::start_long_term_stats;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use stats::{BUS_REQUESTS, FLOWS_TRACKED, HIGH_WATERMARK, TIME_TO_POLL_HOSTS};
use throughput_tracker::flow_data::get_rtt_events_per_second;
use tracing::{error, info, warn};

// Use MiMalloc only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use mimalloc::MiMalloc;

use crate::blackboard::{BLACKBOARD_SENDER, BlackboardCommand};
use crate::remote_commands::start_remote_commands;
#[cfg(feature = "flamegraphs")]
use crate::shaped_devices_tracker::NETWORK_JSON;
#[cfg(feature = "flamegraphs")]
use crate::throughput_tracker::THROUGHPUT_TRACKER;
#[cfg(feature = "flamegraphs")]
use crate::throughput_tracker::flow_data::{ALL_FLOWS, RECENT_FLOWS};
use tracing::level_filters::LevelFilter;
// Use JemAllocator only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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

fn main() -> Result<()> {
    // Set up logging
    set_console_logging()?;

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

    // Spawn tracking sub-systems
    if let Err(e) = lts2_sys::start_lts2() {
        error!("Failed to start Insight: {:?}", e);
    } else {
        info!("Insight client started successfully");
    }
    let _blackboard_tx = blackboard::start_blackboard();
    let long_term_stats_tx = start_long_term_stats();
    start_remote_commands();
    let flow_tx = setup_netflow_tracker()?;
    let _ = throughput_tracker::flow_data::setup_flow_analysis();
    start_heimdall()?;
    spawn_queue_structure_monitor()?;
    shaped_devices_tracker::shaped_devices_watcher()?;
    shaped_devices_tracker::network_json_watcher()?;
    anonymous_usage::start_anonymous_usage();
    let system_usage_tx = system_stats::start_system_stats()?;
    throughput_tracker::spawn_throughput_monitor(
        long_term_stats_tx.clone(),
        flow_tx,
        system_usage_tx.clone(),
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
                        let _ =
                            tokio::runtime::Runtime::new()
                                .unwrap()
                                .block_on(long_term_stats_tx.send(
                                lts_client::collector::stats_availability::StatsUpdateMessage::Quit,
                            ));
                        std::mem::drop(kernels);
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

    let handle = std::thread::Builder::new()
        .name("Async Bus/Web".to_string())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::spawn(async {
                        let _ = lqos_stormguard::start_stormguard().await;
                    });

                    let (bus_tx, bus_rx) = tokio::sync::mpsc::channel::<(
                        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
                        BusRequest,
                    )>(100);

                    // Webserver starting point
                    let webserver_disabled = config.disable_webserver.unwrap_or(false);
                    if !webserver_disabled {
                        tokio::spawn(async {
                            if let Err(e) =
                                node_manager::spawn_webserver(bus_tx, system_usage_tx).await
                            {
                                error!("Node Manager Failed: {e:?}");
                            }
                        });
                    } else {
                        warn!("Webserver disabled by configuration");
                    }

                    // Main bus listen loop
                    server.listen(handle_bus_requests, bus_rx).await.unwrap();
                });
        })?;
    let _ = handle.join();
    warn!("Main thread exiting");
    Ok(())
}

#[cfg(feature = "flamegraphs")]
fn memory_debug() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let mut fb = allocative::FlameGraphBuilder::default();
            fb.visit_global_roots();
            fb.visit_root(&*THROUGHPUT_TRACKER);
            fb.visit_root(&*ALL_FLOWS);
            fb.visit_root(&*RECENT_FLOWS);
            fb.visit_root(&*NETWORK_JSON);
            let flamegraph_src = fb.finish();
            let flamegraph_src = flamegraph_src.flamegraph();
            let Ok(mut file) = std::fs::File::create("/tmp/lqosd-mem.svg") else {
                error!("Unable to write flamegraph.");
                continue;
            };
            file.write_all(flamegraph_src.write().as_bytes()).unwrap();
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
            BusRequest::Ping => lqos_bus::BusResponse::Ack,
            BusRequest::GetCurrentThroughput => throughput_tracker::current_throughput(),
            BusRequest::GetHostCounter => throughput_tracker::host_counters(),
            BusRequest::GetTopNDownloaders { start, end } => {
                throughput_tracker::top_n(*start, *end)
            }
            BusRequest::GetWorstRtt { start, end } => throughput_tracker::worst_n(*start, *end),
            BusRequest::GetWorstRetransmits { start, end } => {
                throughput_tracker::worst_n_retransmits(*start, *end)
            }
            BusRequest::GetBestRtt { start, end } => throughput_tracker::best_n(*start, *end),
            BusRequest::MapIpToFlow {
                ip_address,
                tc_handle,
                cpu,
                upload,
            } => map_ip_to_flow(ip_address, tc_handle, *cpu, *upload),
            BusRequest::ClearHotCache => clear_hot_cache(),
            BusRequest::DelIpFlow { ip_address, upload } => del_ip_flow(ip_address, *upload),
            BusRequest::ClearIpFlow => clear_ip_flows(),
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
            BusRequest::GetLongTermStats(StatsRequest::CurrentTotals) => {
                long_term_stats::get_stats_totals()
            }
            BusRequest::GetLongTermStats(StatsRequest::AllHosts) => {
                long_term_stats::get_stats_host()
            }
            BusRequest::GetLongTermStats(StatsRequest::Tree) => long_term_stats::get_stats_tree(),
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
            BusRequest::InvalidateUserCache => {
                std::thread::spawn(|| {
                    node_manager::invalidate_user_cache_blocking();
                });
                BusResponse::Ack
            }
        });
    }
}
