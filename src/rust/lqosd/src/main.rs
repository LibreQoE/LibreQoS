mod ip_mapping;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
mod throughput_tracker;
mod tuning;
mod program_control;
use crate::{ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow}};
use anyhow::Result;
use log::{info, warn};
use lqos_bus::{
    cookie_value, decode_request, encode_response, BusReply, BusRequest, BUS_BIND_ADDRESS,
};
use lqos_config::LibreQoSConfig;
use lqos_queue_tracker::{spawn_queue_monitor, spawn_queue_structure_monitor, get_raw_circuit_data};
use lqos_sys::LibreQoSKernels;
use signal_hook::{consts::{SIGINT, SIGHUP, SIGTERM }, iterator::Signals};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    join,
    net::{TcpListener, TcpStream}
};

#[tokio::main]
async fn main() -> Result<()> {
    // Configure log level with RUST_LOG environment variable,
    // defaulting to "info"
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "warn")
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
    let mut signals = Signals::new(&[SIGINT, SIGHUP, SIGTERM ])?;
    std::thread::spawn(move || {
        for sig in signals.forever() {
            match sig {
                SIGINT  | SIGTERM  => {
                    match sig {
                        SIGINT => warn!("Terminating on SIGINT"),
                        SIGTERM => warn!("Terminating on SIGTERM"),
                        _ => warn!("This should never happen - terminating on unknown signal"),
                    }
                    std::mem::drop(kernels);
                    std::process::exit(0);        
                }
                SIGHUP => {
                    warn!("Reloading configuration because of SIGHUP");
                    if let Ok(config) = LibreQoSConfig::load() {
                        let result = tuning::tune_lqosd_from_config_file(&config);
                        match result {
                            Err(err) => { warn!("Unable to HUP tunables: {:?}", err) },
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

    // Main bus listen loop
    let listener = TcpListener::bind(BUS_BIND_ADDRESS).await?;
    warn!("Listening on: {}", BUS_BIND_ADDRESS);
    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = vec![0; 1024];

            let _ = socket
                .read(&mut buf)
                .await
                .expect("failed to read data from socket");

            if let Ok(request) = decode_request(&buf) {
                if request.auth_cookie == cookie_value() {
                    let mut response = BusReply {
                        auth_cookie: request.auth_cookie,
                        responses: Vec::new(),
                    };
                    for req in request.requests.iter() {
                        //println!("Request: {:?}", req);
                        response.responses.push(match req {
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
                            BusRequest::UpdateLqosDTuning(..) => {
                                tuning::tune_lqosd_from_bus(&req).await
                            }
                            #[cfg(feature = "equinix_tests")]
                            BusRequest::RequestLqosEquinixTest => {
                                lqos_daht_test::lqos_daht_test().await
                            }
                        });
                    }
                    //println!("{:?}", response);
                    let _ = reply(&encode_response(&response).unwrap(), &mut socket).await;
                }
            }
        });
    }
}

async fn reply(response: &[u8], socket: &mut TcpStream) -> Result<()> {
    socket.write_all(&response).await?;
    Ok(())
}
