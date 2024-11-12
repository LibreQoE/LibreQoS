mod file_lock;
mod ip_mapping;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
mod program_control;
mod shaped_devices_tracker;
mod throughput_tracker;
mod anonymous_usage;
mod tuning;
mod validation;
mod long_term_stats;
mod stats;
mod preflight_checks;
mod system_stats;

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use crate::{
  file_lock::FileLock,
  ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow}, throughput_tracker::flow_data::{flowbee_handle_events, setup_netflow_tracker, FlowActor},
};
use anyhow::Result;
use tracing::{info, warn, error};
use lqos_bus::{BusRequest, BusResponse, UnixSocketServer, StatsRequest, FlowTimeline, CircuitCapacity, FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysisTransport};
use lqos_heimdall::{n_second_packet_dump, perf_interface::heimdall_handle_events, start_heimdall};
use lqos_queue_tracker::{add_watched_queue, get_raw_circuit_data, spawn_queue_monitor, spawn_queue_structure_monitor, TOTAL_QUEUE_STATS};
use lqos_sys::LibreQoSKernels;
use lts_client::collector::start_long_term_stats;
use signal_hook::{
  consts::{SIGHUP, SIGINT, SIGTERM},
  iterator::Signals,
};
use stats::{BUS_REQUESTS, TIME_TO_POLL_HOSTS, HIGH_WATERMARK, FLOWS_TRACKED};
use throughput_tracker::flow_data::get_rtt_events_per_second;
use crate::ip_mapping::clear_hot_cache;

// Use MiMalloc only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use mimalloc::MiMalloc;

use tracing::level_filters::LevelFilter;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use crate::system_stats::{SystemStats, STATS_SENDER};
use crate::throughput_tracker::flow_data::FlowAnalysis;

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
  let file_lock = FileLock::new()
      .inspect_err(|e| {
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
      Some(flowbee_handle_events)
    )?
  };

  // Spawn tracking sub-systems
  let long_term_stats_tx = start_long_term_stats();
  let flow_tx = setup_netflow_tracker()?;
  let _ = throughput_tracker::flow_data::setup_flow_analysis();
  start_heimdall()?;
  spawn_queue_structure_monitor()?;
  shaped_devices_tracker::shaped_devices_watcher()?;
  shaped_devices_tracker::network_json_watcher()?;
  anonymous_usage::start_anonymous_usage();
  throughput_tracker::spawn_throughput_monitor(long_term_stats_tx.clone(), flow_tx)?;
  spawn_queue_monitor()?;
  system_stats::start_system_stats()?;
  let system_usage_tx = system_stats::STATS_SENDER.get().unwrap();
  lqos_sys::bpf_garbage_collector();

  // Handle signals
  let mut signals = Signals::new([SIGINT, SIGHUP, SIGTERM])?;
  std::thread::Builder::new().name("Signal Handler".to_string()).spawn(move || {
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
          let _ = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(long_term_stats_tx.send(lts_client::collector::stats_availability::StatsUpdateMessage::Quit));
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

  let handle = std::thread::Builder::new().name("Async Bus/Web".to_string()).spawn(move || {
    tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build().unwrap()
    .block_on(async {
      // Main bus listen loop
      server.listen(handle_bus_requests).await.unwrap();
    });
  })?;
  let _ = handle.join();
  warn!("Main thread exiting");
  Ok(())
}

fn handle_bus_requests(
  requests: &[BusRequest],
  responses: &mut Vec<BusResponse>,
) {
  for req in requests.iter() {
    //println!("Request: {:?}", req);
    BUS_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    responses.push(match req {
      BusRequest::Ping => lqos_bus::BusResponse::Ack,
      BusRequest::GetCurrentThroughput => {
        throughput_tracker::current_throughput()
      }
      BusRequest::GetHostCounter => throughput_tracker::host_counters(),
      BusRequest::GetTopNDownloaders { start, end } => {
        throughput_tracker::top_n(*start, *end)
      }
      BusRequest::GetWorstRtt { start, end } => {
        throughput_tracker::worst_n(*start, *end)
      }
      BusRequest::GetWorstRetransmits { start, end } => {
        throughput_tracker::worst_n_retransmits(*start, *end)
      }
      BusRequest::GetBestRtt { start, end } => {
        throughput_tracker::best_n(*start, *end)
      }
      BusRequest::MapIpToFlow { ip_address, tc_handle, cpu, upload } => {
        map_ip_to_flow(ip_address, tc_handle, *cpu, *upload)
      }
      BusRequest::ClearHotCache => clear_hot_cache(),
      BusRequest::DelIpFlow { ip_address, upload } => {
        del_ip_flow(ip_address, *upload)
      }
      BusRequest::ClearIpFlow => clear_ip_flows(),
      BusRequest::ListIpFlow => list_mapped_ips(),
      BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
      BusRequest::RttHistogram => throughput_tracker::rtt_histogram::<50>(),
      BusRequest::HostCounts => throughput_tracker::host_counts(),
      BusRequest::AllUnknownIps => throughput_tracker::all_unknown_ips(),
      BusRequest::ReloadLibreQoS => program_control::reload_libre_qos(),
      BusRequest::GetRawQueueData(circuit_id) => {
        get_raw_circuit_data(circuit_id)
      }
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
      },
      #[cfg(feature = "equinix_tests")]
      BusRequest::RequestLqosEquinixTest => lqos_daht_test::lqos_daht_test(),
      BusRequest::ValidateShapedDevicesCsv => {
        validation::validate_shaped_devices_csv()
      }
      BusRequest::GetNetworkMap { parent } => {
        shaped_devices_tracker::get_one_network_map_layer(*parent)
      }
      BusRequest::GetFullNetworkMap => {
        shaped_devices_tracker::get_full_network_map()
      }
      BusRequest::TopMapQueues(n_queues) => {
        shaped_devices_tracker::get_top_n_root_queues(*n_queues)
      }
      BusRequest::GetNodeNamesFromIds(nodes) => {
        shaped_devices_tracker::map_node_names(nodes)
      }
      BusRequest::GetAllCircuits => shaped_devices_tracker::get_all_circuits(),
      BusRequest::GetFunnel { target: parent } => {
        shaped_devices_tracker::get_funnel(parent)
      }
      BusRequest::GetLqosStats => {
        BusResponse::LqosdStats {
          bus_requests: BUS_REQUESTS.load(std::sync::atomic::Ordering::Relaxed),
          time_to_poll_hosts: TIME_TO_POLL_HOSTS.load(std::sync::atomic::Ordering::Relaxed),
          high_watermark: HIGH_WATERMARK.as_down_up(),
          tracked_flows: FLOWS_TRACKED.load(std::sync::atomic::Ordering::Relaxed),
          rtt_events_per_second: get_rtt_events_per_second(),
        }
      }
      BusRequest::GetPacketHeaderDump(id) => {
        BusResponse::PacketDump(n_second_packet_dump(*id))
      }
      BusRequest::GetPcapDump(id) => {
        BusResponse::PcapDump(lqos_heimdall::n_second_pcap(*id))
      }
      BusRequest::GatherPacketData(ip) => {
        let ip = ip.parse::<IpAddr>();
        if let Ok(ip) = ip {
          if let Some((session_id, countdown)) = lqos_heimdall::hyperfocus_on_target(ip.into()) {
            BusResponse::PacketCollectionSession { session_id, countdown }
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
      BusRequest::GetLongTermStats(StatsRequest::Tree) => {
        long_term_stats::get_stats_tree()
      }
      BusRequest::DumpActiveFlows => {
        throughput_tracker::dump_active_flows()
      }
      BusRequest::CountActiveFlows => {
        throughput_tracker::count_active_flows()
      }
      BusRequest::TopFlows { n, flow_type } => throughput_tracker::top_flows(*n, *flow_type),
      BusRequest::FlowsByIp(ip) => throughput_tracker::flows_by_ip(ip),
      BusRequest::CurrentEndpointsByCountry => throughput_tracker::current_endpoints_by_country(),
      BusRequest::CurrentEndpointLatLon => throughput_tracker::current_lat_lon(),
      BusRequest::EtherProtocolSummary => throughput_tracker::ether_protocol_summary(),
      BusRequest::IpProtocolSummary => throughput_tracker::ip_protocol_summary(),
      BusRequest::FlowDuration => throughput_tracker::flow_duration(),
      BusRequest::SystemStatsCpuRam => {
          let joiner = std::thread::spawn(|| {
            if let Some(system_stats_tx) = STATS_SENDER.get() {
              let (tx, rx) = tokio::sync::oneshot::channel();
              let _ = system_stats_tx.send(tx);
              match rx.blocking_recv() {
                Ok(stats) => BusResponse::SystemStatsCpuRam {
                  cpu_usage: stats.cpu_usage,
                  ram_used: stats.ram_used,
                  total_ram: stats.total_ram,
                },
                Err(_) => BusResponse::Fail("System stats not available".to_string())
              }
            } else {
              BusResponse::Fail("System stats not available".to_string())
            }
          }
        );
        joiner.join().unwrap()
      }
      BusRequest::TotalCakeStats => {
        let marks = TOTAL_QUEUE_STATS.marks.as_down_up();
        let drops = TOTAL_QUEUE_STATS.drops.as_down_up();
        BusResponse::TotalCakeStats { marks, drops }
      }
      BusRequest::UnknownIps => {
        throughput_tracker::unknown_ips()
      }
      BusRequest::FlowLatLon => {
        BusResponse::FlowLatLon(crate::throughput_tracker::flow_data::RECENT_FLOWS.lat_lon_endpoints())
      }
      BusRequest::FlowAsnList => {
        BusResponse::FlowAsnList(
          crate::throughput_tracker::flow_data::RECENT_FLOWS.asn_list()
        )
      }
      BusRequest::FlowCountryList => {
        BusResponse::FlowCountryList(
          crate::throughput_tracker::flow_data::RECENT_FLOWS.country_list()
        )
      }
      BusRequest::FlowProtocolList => {
        BusResponse::FlowProtocolList(
          crate::throughput_tracker::flow_data::RECENT_FLOWS.protocol_list()
        )
      }
      BusRequest::FlowTimeline(asn_id) => flow_timeline(*asn_id),
      BusRequest::FlowCountryTimeline(iso_code) => flow_country_timeline(iso_code),
      BusRequest::FlowProtocolTimeline(protocol_name) => flow_protocol_timeline(protocol_name),
      BusRequest::CircuitCapacities => {
        BusResponse::CircuitCapacities(circuit_capacity())
      }
      BusRequest::FlowsByCircuit(circuit) => {
        let flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysisTransport)> = recent_flows_by_circuit(&circuit)
            .into_iter()
            .map(|(key, local, analysis)| {
              (key.into(), local, analysis)
            })
            .collect();
        BusResponse::FlowsByCircuit(flows)
      }
    });
  }
}

fn flow_protocol_timeline(protocol_name: &str) -> BusResponse {
    let Ok(time_since_boot) = time_since_boot() else {
        return BusResponse::Fail("Unable to get time since boot".to_string());
    };
    let since_boot = Duration::from(time_since_boot);
    let Ok(unix_now) = unix_now() else {
        return BusResponse::Fail("Unable to get current time".to_string());
    };
    let boot_time = unix_now - since_boot.as_secs();

    let all_flows_for_asn = crate::throughput_tracker::flow_data::RECENT_FLOWS.all_flows_for_protocol(protocol_name);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    BusResponse::FlowTimeline(flows)
}

fn flow_timeline(asn_id: u32) -> BusResponse {
  let Ok(time_since_boot) = time_since_boot() else {
    return BusResponse::Fail("Unable to get time since boot".to_string());
  };
  let since_boot = Duration::from(time_since_boot);
  let Ok(unix_now) = unix_now() else {
    return BusResponse::Fail("Unable to get current time".to_string());
  };
  let boot_time = unix_now - since_boot.as_secs();

  let all_flows_for_asn = crate::throughput_tracker::flow_data::RECENT_FLOWS.all_flows_for_asn(asn_id);

  let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

  BusResponse::FlowTimeline(flows)
}

fn flow_country_timeline(iso_code: &str) -> BusResponse {
    let Ok(time_since_boot) = time_since_boot() else {
        return BusResponse::Fail("Unable to get time since boot".to_string());
    };
    let since_boot = Duration::from(time_since_boot);
    let Ok(unix_now) = unix_now() else {
        return BusResponse::Fail("Unable to get current time".to_string());
    };
    let boot_time = unix_now - since_boot.as_secs();

    let all_flows_for_asn = crate::throughput_tracker::flow_data::RECENT_FLOWS.all_flows_for_country(iso_code);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    BusResponse::FlowCountryTimeline(flows)
}

fn all_flows_to_transport(boot_time: u64, all_flows_for_asn: Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)>) -> Vec<FlowTimeline> {
  all_flows_for_asn
      .iter()
      .filter(|flow| {
        // Total flow time > 2 seconds
        flow.1.last_seen - flow.1.start_time > 2_000_000_000
      })
      .map(|flow| {
        let (circuit_id, mut circuit_name) = {
          let sd = crate::shaped_devices_tracker::SHAPED_DEVICES.load();
          sd.get_circuit_id_and_name_from_ip(&flow.0.local_ip).unwrap_or((String::new(), String::new()))
        };
        if circuit_name.is_empty() {
          circuit_name = flow.0.local_ip.as_ip().to_string();
        }

        FlowTimeline {
          start: boot_time + Duration::from_nanos(flow.1.start_time).as_secs(),
          end: boot_time + Duration::from_nanos(flow.1.last_seen).as_secs(),
          duration_nanos: flow.1.last_seen - flow.1.start_time,
          tcp_retransmits: flow.1.tcp_retransmits.clone(),
          throughput: flow.1.throughput_buffer.clone(),
          rtt: flow.1.rtt.clone(),
          retransmit_times_down: flow.1.retry_times_down
              .iter()
              .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
              .collect(),
          retransmit_times_up: flow.1.retry_times_up
              .iter()
              .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
              .collect(),
          total_bytes: flow.1.bytes_sent.clone(),
          protocol: flow.2.protocol_analysis.to_string(),
          circuit_id,
          circuit_name,
          remote_ip: flow.0.remote_ip.as_ip().to_string(),
        }
      })
      .collect::<Vec<_>>()
}

struct CircuitAccumulator {
  bytes: lqos_utils::units::DownUpOrder<u64>,
  median_rtt: f32,
}

fn circuit_capacity() -> Vec<CircuitCapacity> {
  let mut circuits: HashMap<String, CircuitAccumulator> = HashMap::new();

  // Aggregate the data by circuit id
  crate::throughput_tracker::THROUGHPUT_TRACKER.raw_data.iter().for_each(|c| {
    if let Some(circuit_id) = &c.circuit_id {
      if let Some(accumulator) = circuits.get_mut(circuit_id) {
        accumulator.bytes += c.bytes_per_second;
        if let Some(latency) = c.median_latency() {
          accumulator.median_rtt = latency;
        }
      } else {
        circuits.insert(circuit_id.clone(), CircuitAccumulator {
          bytes: c.bytes_per_second,
          median_rtt: c.median_latency().unwrap_or(0.0),
        });
      }
    }
  });

  // Map circuits to capacities
  let shaped_devices = crate::shaped_devices_tracker::SHAPED_DEVICES.load();
  let capacities: Vec<CircuitCapacity> = {
    circuits.iter().filter_map(|(circuit_id, accumulator)| {
      if let Some(device) = shaped_devices.devices.iter().find(|sd| sd.circuit_id == *circuit_id) {
        let down_mbps = (accumulator.bytes.down as f64 * 8.0) / 1_000_000.0;
        let down = down_mbps / device.download_max_mbps as f64;
        let up_mbps = (accumulator.bytes.up as f64 * 8.0) / 1_000_000.0;
        let up = up_mbps / device.upload_max_mbps as f64;

        Some(CircuitCapacity {
          circuit_name: device.circuit_name.clone(),
          circuit_id: circuit_id.clone(),
          capacity: [down, up],
          median_rtt: accumulator.median_rtt,
        })
      } else {
        None
      }
    }).collect()
  };
  return capacities;
}

fn recent_flows_by_circuit(circuit_id: &str) -> Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysisTransport)> {
  const FIVE_MINUTES_AS_NANOS: u64 = 300 * 1_000_000_000;

  let device_reader = crate::shaped_devices_tracker::SHAPED_DEVICES.load();
  if let Ok(now) = time_since_boot() {
    let now_as_nanos = Duration::from(now).as_nanos() as u64;
    let five_minutes_ago = now_as_nanos - FIVE_MINUTES_AS_NANOS;

    if let Ok(all_flows) = crate::throughput_tracker::flow_data::ALL_FLOWS.lock() {
      let result: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> = all_flows
          .iter()
          .filter_map(|(key, (local, analysis))| {
            // Don't show older flows
            if local.last_seen < five_minutes_ago {
              return None;
            }

            // Don't show flows that don't belong to the circuit
            let local_ip_str; // Using late binding
            let remote_ip_str;
            let device_name;
            let asn_name;
            let asn_country;
            let local_ip = match key.local_ip.as_ip() {
              IpAddr::V4(ip) => ip.to_ipv6_mapped(),
              IpAddr::V6(ip) => ip,
            };
            let remote_ip = match key.remote_ip.as_ip() {
              IpAddr::V4(ip) => ip.to_ipv6_mapped(),
              IpAddr::V6(ip) => ip,
            };
            if let Some(device) = device_reader.trie.longest_match(local_ip) {
              // The normal way around
              local_ip_str = key.local_ip.to_string();
              remote_ip_str = key.remote_ip.to_string();
              let device = &device_reader.devices[*device.1];
              if device.circuit_id != circuit_id {
                return None;
              }
              device_name = device.device_name.clone();
              let geo = crate::throughput_tracker::flow_data::get_asn_name_and_country(key.remote_ip.as_ip());
              (asn_name, asn_country) = (geo.name, geo.country);
            } else if let Some(device) = device_reader.trie.longest_match(remote_ip) {
              // The reverse way around
              local_ip_str = key.remote_ip.to_string();
              remote_ip_str = key.local_ip.to_string();
              let device = &device_reader.devices[*device.1];
              if device.circuit_id != circuit_id {
                return None;
              }
              device_name = device.device_name.clone();
              let geo = crate::throughput_tracker::flow_data::get_asn_name_and_country(key.local_ip.as_ip());
              (asn_name, asn_country) = (geo.name, geo.country);
            } else {
              return None;
            }

            Some((FlowbeeKeyTransit {
              remote_ip: remote_ip_str.parse().unwrap(),
              local_ip: local_ip_str.parse().unwrap(),
              src_port: key.src_port,
              dst_port: key.dst_port,
              ip_protocol: key.ip_protocol,
            }, local.clone(), analysis.clone()))
          })
          .collect();

      let result = result
          .into_iter()
          .map(|(key, local, analysis)| {
            (key, local, FlowAnalysisTransport {
              asn_id: analysis.asn_id,
              protocol_analysis: analysis.protocol_analysis,
            })
          })
          .collect();

      return result;
    }
  }
  Vec::new()
}