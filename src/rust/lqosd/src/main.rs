mod ip_mapping;
mod throughput_tracker;
mod program_control;
mod queue_tracker;
mod libreqos_tracker;
#[cfg(feature = "equinix_tests")]
mod lqos_daht_test;
mod offloads;
use crate::ip_mapping::{clear_ip_flows, del_ip_flow, list_mapped_ips, map_ip_to_flow};
use anyhow::Result;
use lqos_bus::{
    cookie_value, decode_request, encode_response, BusReply, BusRequest, BUS_BIND_ADDRESS,
};
use lqos_config::{LibreQoSConfig, EtcLqos};
use lqos_sys::LibreQoSKernels;
use signal_hook::{consts::SIGINT, iterator::Signals};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream}, join,
};
use log::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init(); // Configure log level with RUST_LOG environment variable
    info!("LibreQoS Daemon Starting");
    let config = LibreQoSConfig::load()?;
    let etc_lqos = EtcLqos::load()?;

    // Disable offloading
    if let Some(tuning) = &etc_lqos.tuning {
        offloads::bpf_sysctls().await;
        if tuning.stop_irq_balance {
            offloads::stop_irq_balance().await;
        }        
        offloads::netdev_budget(tuning.netdev_budget_usecs, tuning.netdev_budget_packets).await;
        offloads::ethtool_tweaks(&config.internet_interface, tuning).await;
        offloads::ethtool_tweaks(&config.isp_interface, tuning).await;
    }

    // Start the XDP/TC kernels
    let kernels = if config.on_a_stick_mode {
        LibreQoSKernels::on_a_stick_mode(&config.internet_interface, config.stick_vlans.1, config.stick_vlans.0)?
    } else {
        LibreQoSKernels::new(&config.internet_interface, &config.isp_interface)?
    };

    // Spawn tracking sub-systems
    join!(
        throughput_tracker::spawn_throughput_monitor(),
        queue_tracker::spawn_queue_monitor(),
        //libreqos_tracker::spawn_shaped_devices_monitor(),
        libreqos_tracker::spawn_queue_structure_monitor(),
    );

    let mut signals = Signals::new(&[SIGINT])?;

    std::thread::spawn(move || {
        for sig in signals.forever() {
            warn!("Received signal {:?}", sig);
            std::mem::drop(kernels);
            std::process::exit(0);
        }
    });

    // Main bus listen loop
    let listener = TcpListener::bind(BUS_BIND_ADDRESS).await?;
    info!("Listening on: {}", BUS_BIND_ADDRESS);
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
                            BusRequest::GetTopNDownloaders(n) => throughput_tracker::top_n(*n),
                            BusRequest::GetWorstRtt(n) => throughput_tracker::worst_n(*n),
                            BusRequest::MapIpToFlow {
                                ip_address,
                                tc_handle,
                                cpu,
                                upload,
                            } => map_ip_to_flow(ip_address, tc_handle, *cpu, *upload),
                            BusRequest::DelIpFlow { ip_address, upload } => del_ip_flow(&ip_address, *upload),
                            BusRequest::ClearIpFlow => clear_ip_flows(),
                            BusRequest::ListIpFlow => list_mapped_ips(),
                            BusRequest::XdpPping => throughput_tracker::xdp_pping_compat(),
                            BusRequest::RttHistogram => throughput_tracker::rtt_histogram(),
                            BusRequest::HostCounts => throughput_tracker::host_counts(),
                            BusRequest::AllUnknownIps => throughput_tracker::all_unknown_ips(),
                            BusRequest::ReloadLibreQoS => program_control::reload_libre_qos(),
                            BusRequest::GetRawQueueData(circuit_id) => queue_tracker::get_raw_circuit_data(&circuit_id),
                            #[cfg(feature = "equinix_tests")]
                            BusRequest::RequestLqosEquinixTest => lqos_daht_test::lqos_daht_test().await,
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
