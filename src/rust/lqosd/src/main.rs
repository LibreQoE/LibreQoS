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
use std::net::IpAddr;

use crate::{
  file_lock::FileLock,
  ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow},
};
use anyhow::Result;
use log::{info, warn};
use lqos_bus::{BusRequest, BusResponse, UnixSocketServer};
use lqos_config::LibreQoSConfig;
use lqos_heimdall::{n_second_packet_dump, perf_interface::heimdall_handle_events, start_heimdall};
use lqos_queue_tracker::{
  add_watched_queue, get_raw_circuit_data, spawn_queue_monitor,
  spawn_queue_structure_monitor,
};
use lqos_sys::LibreQoSKernels;
use signal_hook::{
  consts::{SIGHUP, SIGINT, SIGTERM},
  iterator::Signals,
};
use stats::{BUS_REQUESTS, TIME_TO_POLL_HOSTS, HIGH_WATERMARK_DOWN, HIGH_WATERMARK_UP, FLOWS_TRACKED};
use throughput_tracker::get_flow_stats;
use tokio::join;
mod stats;

// Use JemAllocator only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use jemallocator::Jemalloc;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<()> {
  // Configure log level with RUST_LOG environment variable,
  // defaulting to "warn"
  env_logger::init_from_env(
    env_logger::Env::default()
      .filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
  );
  let file_lock = FileLock::new();
  if let Err(e) = file_lock {
    log::error!("File lock error: {:?}", e);
    std::process::exit(0);
  }

  info!("LibreQoS Daemon Starting");
  let config = LibreQoSConfig::load()?;
  tuning::tune_lqosd_from_config_file(&config)?;

  // Start the XDP/TC kernels
  let kernels = if config.on_a_stick_mode {
    LibreQoSKernels::on_a_stick_mode(
      &config.internet_interface,
      config.stick_vlans.1,
      config.stick_vlans.0,
      Some(heimdall_handle_events),
    )?
  } else {
    LibreQoSKernels::new(&config.internet_interface, &config.isp_interface, Some(heimdall_handle_events))?
  };

  // Spawn tracking sub-systems
  join!(
    start_heimdall(),
    spawn_queue_structure_monitor(),
    shaped_devices_tracker::shaped_devices_watcher(),
    shaped_devices_tracker::network_json_watcher(),
    anonymous_usage::start_anonymous_usage(),
  );
  throughput_tracker::spawn_throughput_monitor();
  spawn_queue_monitor();

  // Handle signals
  let mut signals = Signals::new([SIGINT, SIGHUP, SIGTERM])?;
  std::thread::spawn(move || {
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
          UnixSocketServer::signal_cleanup();
          std::mem::drop(file_lock);
          std::process::exit(0);
        }
        SIGHUP => {
          warn!("Reloading configuration because of SIGHUP");
          if let Ok(config) = LibreQoSConfig::load() {
            let result = tuning::tune_lqosd_from_config_file(&config);
            if let Err(err) = result {
              warn!("Unable to HUP tunables: {:?}", err)
            }
          } else {
            warn!("Unable to reload configuration");
          }
        }
        _ => warn!("No handler for signal: {sig}"),
      }
    }
  });

  // Create the socket server
  let server = UnixSocketServer::new().expect("Unable to spawn server");

  // Main bus listen loop
  server.listen(handle_bus_requests).await?;
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
      BusRequest::GetBestRtt { start, end } => {
        throughput_tracker::best_n(*start, *end)
      }
      BusRequest::MapIpToFlow { ip_address, tc_handle, cpu, upload } => {
        map_ip_to_flow(ip_address, tc_handle, *cpu, *upload)
      }
      BusRequest::DelIpFlow { ip_address, upload } => {
        del_ip_flow(ip_address, *upload)
      }
      BusRequest::ClearIpFlow => clear_ip_flows(),
      BusRequest::ListIpFlow => list_mapped_ips(),
      BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
      BusRequest::RttHistogram => throughput_tracker::rtt_histogram(),
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
      #[cfg(feature = "equinix_tests")]
      BusRequest::RequestLqosEquinixTest => lqos_daht_test::lqos_daht_test(),
      BusRequest::ValidateShapedDevicesCsv => {
        validation::validate_shaped_devices_csv()
      }
      BusRequest::GetNetworkMap { parent } => {
        shaped_devices_tracker::get_one_network_map_layer(*parent)
      }
      BusRequest::TopMapQueues(n_queues) => {
        shaped_devices_tracker::get_top_n_root_queues(*n_queues)
      }
      BusRequest::GetNodeNamesFromIds(nodes) => {
        shaped_devices_tracker::map_node_names(nodes)
      }
      BusRequest::GetFunnel { target: parent } => {
        shaped_devices_tracker::get_funnel(parent)
      }
      BusRequest::GetLqosStats => {
        BusResponse::LqosdStats { 
          bus_requests: BUS_REQUESTS.load(std::sync::atomic::Ordering::Relaxed),
          time_to_poll_hosts: TIME_TO_POLL_HOSTS.load(std::sync::atomic::Ordering::Relaxed),
          high_watermark: (
            HIGH_WATERMARK_DOWN.load(std::sync::atomic::Ordering::Relaxed),
            HIGH_WATERMARK_UP.load(std::sync::atomic::Ordering::Relaxed),
          ),
          tracked_flows: FLOWS_TRACKED.load(std::sync::atomic::Ordering::Relaxed),
        }
      }
      BusRequest::GetFlowStats(ip) => get_flow_stats(ip),
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
            BusResponse::PacketCollectionSession{session_id, countdown}
          } else {
            BusResponse::Fail("Busy".to_string())
          }
        } else {
          BusResponse::Fail("Invalid IP".to_string())
        }
      }
    });
  }
}
