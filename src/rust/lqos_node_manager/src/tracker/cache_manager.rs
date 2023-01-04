//! The Cache mod stores data that is periodically updated
//! on the server-side, to avoid re-requesting repeatedly
//! when there are multiple clients.
use std::{time::Duration, net::IpAddr};
use anyhow::Result;
use lqos_bus::{BUS_BIND_ADDRESS, BusSession, BusRequest, encode_request, BusResponse, decode_response, IpStats};
use lqos_config::ConfigShapedDevices;
use rocket::tokio::{task::spawn_blocking, net::TcpStream, io::{AsyncWriteExt, AsyncReadExt}};
use super::cache::*;

/// Once per second, update CPU and RAM usage and ask
/// `lqosd` for updated system statistics.
/// Called from the main program as a "fairing", meaning
/// it runs as part of start-up - and keeps running.
/// Designed to never return or fail on error.
pub async fn update_tracking() {
    use sysinfo::System;
    use sysinfo::CpuExt;
    use sysinfo::SystemExt;
    let mut sys = System::new_all();

    spawn_blocking(|| {
        let _ = watch_for_shaped_devices_changing();
    });

    loop {
        //println!("Updating tracking data");
        sys.refresh_cpu();
        sys.refresh_memory();
        let cpu_usage = sys
            .cpus()
            .iter()
            .map(|cpu| cpu.cpu_usage())
            .collect::<Vec<f32>>();
        *CPU_USAGE.write() = cpu_usage;
        {
            let mut mem_use = MEMORY_USAGE.write();
            mem_use[0] = sys.used_memory();
            mem_use[1] = sys.total_memory();
        }
        let _ = get_data_from_server().await; // Ignoring errors to keep running
        rocket::tokio::time::sleep(Duration::from_secs(1)).await;
    }
}


/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
    use notify::{Watcher, RecursiveMode, Config};

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())?;

    watcher.watch(&ConfigShapedDevices::path()?, RecursiveMode::NonRecursive)?;
    loop {
        let _ = rx.recv();
        if let Ok(new_file) = ConfigShapedDevices::load() {
            println!("ShapedDevices.csv changed");
            *SHAPED_DEVICES.write() = new_file;
        }
    }
}

/// Requests data from `lqosd` and stores it in local
/// caches.
async fn get_data_from_server() -> Result<()> {
    // Send request to lqosd
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await?;
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![
            BusRequest::GetCurrentThroughput,
            BusRequest::GetTopNDownloaders(10),
            BusRequest::GetWorstRtt(10),
            BusRequest::RttHistogram,
            BusRequest::AllUnknownIps,
        ],
    };
    let msg = encode_request(&test)?;
    stream.write(&msg).await?;

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf)?;

    // Process the reply
    for r in reply.responses.iter() {
        match r {
            BusResponse::CurrentThroughput {
                bits_per_second,
                packets_per_second,
                shaped_bits_per_second
            } => {
                {
                    let mut lock = CURRENT_THROUGHPUT.write();
                    lock.bits_per_second = *bits_per_second;
                    lock.packets_per_second = *packets_per_second;
                } // Lock scope
                {
                    let mut lock = THROUGHPUT_BUFFER.write();
                    lock.store(ThroughputPerSecond {
                        packets_per_second: *packets_per_second,
                        bits_per_second: *bits_per_second,
                        shaped_bits_per_second: *shaped_bits_per_second,
                    });
                }
            }
            BusResponse::TopDownloaders(stats) => {
                *TOP_10_DOWNLOADERS.write() = stats.clone();
            }
            BusResponse::WorstRtt(stats) => {
                *WORST_10_RTT.write() = stats.clone();
            }
            BusResponse::RttHistogram(stats) => {
                *RTT_HISTOGRAM.write() = stats.clone();
            }
            BusResponse::AllUnknownIps(unknowns) => {
                *HOST_COUNTS.write() = (unknowns.len() as u32, 0);
                let cfg = SHAPED_DEVICES.read();
                let really_unknown: Vec<IpStats> = unknowns.iter().filter(|ip| {
                    if let Ok(ip) = ip.ip_address.parse::<IpAddr>() {
                        let lookup = match ip {
                            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                            IpAddr::V6(ip) => ip,
                        };
                        cfg.trie.longest_match(lookup).is_none()
                    } else {
                        false
                    }
                }).cloned().collect();
                *HOST_COUNTS.write() = (really_unknown.len() as u32, 0);
                *UNKNOWN_DEVICES.write() = really_unknown;
            }
            // Default
            _ => {}
        }
    }

    Ok(())
}