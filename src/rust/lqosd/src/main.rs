mod file_lock;
mod ip_mapping;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
mod program_control;
mod throughput_tracker;
mod tuning;
mod validation;
use crate::{
  file_lock::FileLock,
  ip_mapping::{
    clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow,
  },
};
use anyhow::Result;
use log::{info, warn};
use lqos_bus::{BusRequest, BusResponse, UnixSocketServer};
use lqos_config::LibreQoSConfig;
use lqos_queue_tracker::{
  add_watched_queue, get_raw_circuit_data, spawn_queue_monitor,
  spawn_queue_structure_monitor,
};
use lqos_sys::LibreQoSKernels;
use signal_hook::{
  consts::{SIGHUP, SIGINT, SIGTERM},
  iterator::Signals,
};
use tokio::join;

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
    )?
  } else {
    LibreQoSKernels::new(&config.internet_interface, &config.isp_interface)?
  };

  // Spawn tracking sub-systems
  join!(spawn_queue_structure_monitor(),);
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
            _ => warn!(
              "This should never happen - terminating on unknown signal"
            ),
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

  // Main bus listen loop
  let listener = async move {
    // Create the socket server
    let server = UnixSocketServer::new().expect("Unable to spawn server");
    server.listen(handle_bus_requests).await
  };
  let handle = tokio::spawn(listener);

  if lqos_config::LibreQoSConfig::config_exists() && lqos_config::ConfigShapedDevices::exists() {
    warn!("Since all the files exist, Launching LibreQoS.py to avoid empty queues.");
    program_control::reload_libre_qos();
  } else {
    warn!("ispConfig.py or ShapedDevices.csv hasn't been setup yet. Not automatically running LibreQoS.py");
  }

  info!("{:?}", handle.await?);

  Ok(())
}

fn handle_bus_requests(
  requests: &[BusRequest],
  responses: &mut Vec<BusResponse>,
) {
  for req in requests.iter() {
    //println!("Request: {:?}", req);
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
    });
  }
}
