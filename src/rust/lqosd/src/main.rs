mod ip_mapping;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
mod program_control;
mod throughput_tracker;
mod tuning;
use crate::{ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow}};
use anyhow::Result;
use log::{info, warn};
use lqos_bus::{BusResponse, BusRequest, UnixSocketServer};
use lqos_config::LibreQoSConfig;
use lqos_queue_tracker::{
    add_watched_queue, get_raw_circuit_data, spawn_queue_monitor, spawn_queue_structure_monitor,
};
use lqos_sys::LibreQoSKernels;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use tokio::join;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure log level with RUST_LOG environment variable,
    // defaulting to "warn"
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
    );
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
    join!(
        throughput_tracker::spawn_throughput_monitor(),
        spawn_queue_monitor(),
        spawn_queue_structure_monitor(),
    );

    // Handle signals
    let mut signals = Signals::new(&[SIGINT, SIGHUP, SIGTERM])?;
    std::thread::spawn(move || {
        for sig in signals.forever() {
            match sig {
                SIGINT | SIGTERM => {
                    match sig {
                        SIGINT => warn!("Terminating on SIGINT"),
                        SIGTERM => warn!("Terminating on SIGTERM"),
                        _ => warn!("This should never happen - terminating on unknown signal"),
                    }
                    std::mem::drop(kernels);
                    UnixSocketServer::signal_cleanup();
                    std::process::exit(0);
                }
                SIGHUP => {
                    warn!("Reloading configuration because of SIGHUP");
                    if let Ok(config) = LibreQoSConfig::load() {
                        let result = tuning::tune_lqosd_from_config_file(&config);
                        match result {
                            Err(err) => {
                                warn!("Unable to HUP tunables: {:?}", err)
                            }
                            Ok(..) => {}
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

fn handle_bus_requests(requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
    for req in requests.iter() {
        //println!("Request: {:?}", req);
        responses.push(match req {
            BusRequest::Ping => lqos_bus::BusResponse::Ack,
            BusRequest::GetCurrentThroughput => {
                throughput_tracker::current_throughput()
            }
            BusRequest::GetHostCounter => throughput_tracker::host_counters(),
            BusRequest::GetTopNDownloaders(n) => throughput_tracker::top_n(*n),
            BusRequest::GetWorstRtt(n) => throughput_tracker::worst_n(*n),
            BusRequest::MapIpToFlow {
                ip_address,
                tc_handle,
                cpu,
                upload,
            } => map_ip_to_flow(ip_address, tc_handle, *cpu, *upload),
            BusRequest::DelIpFlow { ip_address, upload } => {
                del_ip_flow(&ip_address, *upload)
            }
            BusRequest::ClearIpFlow => clear_ip_flows(),
            BusRequest::ListIpFlow => list_mapped_ips(),
            BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
            BusRequest::RttHistogram => throughput_tracker::rtt_histogram(),
            BusRequest::HostCounts => throughput_tracker::host_counts(),
            BusRequest::AllUnknownIps => throughput_tracker::all_unknown_ips(),
            BusRequest::ReloadLibreQoS => program_control::reload_libre_qos(),
            BusRequest::GetRawQueueData(circuit_id) => {
                get_raw_circuit_data(&circuit_id)
            }
            BusRequest::WatchQueue(circuit_id) => {
                add_watched_queue(&circuit_id);
                lqos_bus::BusResponse::Ack
            }
            BusRequest::UpdateLqosDTuning(..) => {
                tuning::tune_lqosd_from_bus(&req)
            }
            #[cfg(feature = "equinix_tests")]
            BusRequest::RequestLqosEquinixTest => {
                lqos_daht_test::lqos_daht_test()
            }
        });
    }
}