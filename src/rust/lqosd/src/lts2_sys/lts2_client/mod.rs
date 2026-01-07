//#![warn(missing_docs)]
//! LTS Client. This is designed to be dynamically linked and loaded at runtime in
//! `lqosd`. As such, it exposes a relatively simple API.
//!
//! Since Rust lacks a stable ABI, the API is defined as a stable C ABI. This reduces
//! the range of things we can do - but means we won't run into "DLL Hell" issues.

mod client_commands;
mod free_trial;
mod ingestor;
mod license_check;
mod nacl_blob;
mod remote_commands;

use crate::lts2_sys::shared_types;
use crate::lts2_sys::shared_types::{CircuitRetransmits, FreeTrialDetails};
use client_commands::LtsClientCommand;
pub(crate) use license_check::{LicenseStatus, get_license_status, set_license_status};
use lqos_config::load_config;
pub(crate) use remote_commands::enqueue;
use std::net::IpAddr;
use std::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{error, warn};

pub fn spawn_lts2(
    control_tx: tokio::sync::mpsc::Sender<super::control_channel::ControlChannelCommand>,
) -> anyhow::Result<()> {
    // Channel construction
    let (tx, rx) = mpsc::channel::<LtsClientCommand>();
    client_commands::set_command_channel(tx)?;

    // Launch the ingestor thread
    let ingestor = ingestor::start_ingestor();

    // Message Pump
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                LtsClientCommand::RequestFreeTrial(trial, channel) => {
                    free_trial::request_free_trial(trial, channel);
                }
                LtsClientCommand::IngestData(data) => {
                    if let Err(e) = ingestor.send(data) {
                        warn!("Failed to send data to ingestor: {:?}", e);
                    }
                }
                LtsClientCommand::LicenseStatus(channel) => {
                    let status = get_license_status();
                    let _ = channel.send(status.license_type);
                }
                LtsClientCommand::TrialDaysRemaining(channel) => {
                    let status = get_license_status();
                    let _ = channel.send(status.trial_expires);
                }
                LtsClientCommand::IngestBatchComplete => {
                    // Submit to the unified channel system
                    let _ =
                        ingestor.send(ingestor::commands::IngestorCommand::IngestBatchComplete {
                            submit: control_tx.clone(),
                        });
                }
            }
        }
        warn!("Insight Client message pump exited");
    });

    Ok(()) // Success
}

pub fn request_free_trial(details: FreeTrialDetails) -> anyhow::Result<String> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let (otx, orx) = oneshot::channel();
        if tx
            .send(LtsClientCommand::RequestFreeTrial(details, otx))
            .is_err()
        {
            println!("Failed to send free trial request to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send free trial request to LTS2 client"
            ));
        }
        return if let Ok(result) = orx.blocking_recv() {
            Ok(result)
        } else {
            println!("Failed to receive free trial response from LTS2 client");
            Err(anyhow::anyhow!(
                "Failed to receive free trial response from LTS2 client"
            ))
        };
    }
    Err(anyhow::anyhow!("Failed to get command channel"))
}

pub fn submit_network_tree(timestamp: u64, tree: &[u8]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let tree = tree.to_vec();
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::NetworkTree { timestamp, tree },
            ))
            .is_err()
        {
            println!("Failed to send network tree to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send network tree to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_shaped_devices(timestamp: u64, devices: &[u8]) -> anyhow::Result<()> {
    let devices = devices.to_vec();
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::ShapedDevices { timestamp, devices },
            ))
            .is_err()
        {
            println!("Failed to send shaped devices to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send shaped devices to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_total_throughput(
    timestamp: u64,
    download_bytes: u64,
    upload_bytes: u64,
    shaped_download_bytes: u64,
    shaped_upload_bytes: u64,
    packets_down: u64,
    packets_up: u64,
    tcp_packets_down: u64,
    tcp_packets_up: u64,
    udp_packets_down: u64,
    udp_packets_up: u64,
    icmp_packets_down: u64,
    icmp_packets_up: u64,
    has_max_rtt: bool,
    max_rtt: f32,
    has_min_rtt: bool,
    min_rtt: f32,
    has_median_rtt: bool,
    median_rtt: f32,
    tcp_retransmits_down: i32,
    tcp_retransmits_up: i32,
    cake_marks_down: i32,
    cake_marks_up: i32,
    cake_drops_down: i32,
    cake_drops_up: i32,
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let max_rtt = if has_max_rtt { Some(max_rtt) } else { None };
        let min_rtt = if has_min_rtt { Some(min_rtt) } else { None };
        let median_rtt = if has_median_rtt {
            Some(median_rtt)
        } else {
            None
        };
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::TotalThroughput {
                    timestamp,
                    download_bytes,
                    upload_bytes,
                    shaped_download_bytes,
                    shaped_upload_bytes,
                    packets_down,
                    packets_up,
                    tcp_packets_down,
                    tcp_packets_up,
                    udp_packets_down,
                    udp_packets_up,
                    icmp_packets_down,
                    icmp_packets_up,
                    max_rtt,
                    min_rtt,
                    median_rtt,
                    tcp_retransmits_down,
                    tcp_retransmits_up,
                    cake_marks_down,
                    cake_marks_up,
                    cake_drops_down,
                    cake_drops_up,
                },
            ))
            .is_err()
        {
            println!("Failed to send total throughput to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send total throughput to LTS2 client"
            ));
        }
    } else {
        println!("Failed to get command channel");
        return Err(anyhow::anyhow!("Failed to get command channel"));
    }
    Ok(()) // SUCCESS
}

pub fn submit_shaper_utilization(
    timestamp: u64,
    average_cpu: f32,
    peak_cpu: f32,
    memory_percent: f32,
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if let Err(e) = tx.send(LtsClientCommand::IngestData(
            ingestor::commands::IngestorCommand::ShaperUtilization {
                tick: timestamp,
                average_cpu,
                peak_cpu,
                memory_percent,
            },
        )) {
            println!("Failed to send shaper utilization to LTS2 client: {e:?}");
            return Err(anyhow::anyhow!(
                "Failed to send shaper utilization to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_circuit_throughput_batch(
    batch: &[crate::lts2_sys::shared_types::CircuitThroughput],
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::CircuitThroughputBatch(batch.to_vec()),
            ))
            .is_err()
        {
            error!("Failed to send circuit throughput batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send circuit throughput batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_circuit_retransmits_batch(batch: &[CircuitRetransmits]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::CircuitRetransmitsBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send circuit retransmits batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send circuit retransmits batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_circuit_rtt_batch(
    batch: &[crate::lts2_sys::shared_types::CircuitRtt],
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::CircuitRttBatch(batch.to_vec()),
            ))
            .is_err()
        {
            error!("Failed to send circuit RTT batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send circuit RTT batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_circuit_cake_drops_batch(
    batch: &[crate::lts2_sys::shared_types::CircuitCakeDrops],
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::CircuitCakeDropsBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send circuit cake drops batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send circuit cake drops batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_circuit_cake_marks_batch(
    batch: &[crate::lts2_sys::shared_types::CircuitCakeMarks],
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::CircuitCakeMarksBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send circuit cake marks batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send circuit cake marks batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_site_throughput_batch(batch: &[shared_types::SiteThroughput]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::SiteThroughputBatch(batch.to_vec()),
            ))
            .is_err()
        {
            error!("Failed to send site throughput batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send site throughput batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_site_retransmits_batch(
    batch: &[shared_types::SiteRetransmits],
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::SiteRetransmitsBatch(batch.to_vec()),
            ))
            .is_err()
        {
            error!("Failed to send site retransmits batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send site retransmits batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_site_cake_drops_batch(batch: &[shared_types::SiteCakeDrops]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::SiteCakeDropsBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send site cake drops batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send site cake drops batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_site_cake_marks_batch(batch: &[shared_types::SiteCakeMarks]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::SiteCakeMarksBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send site cake marks batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send site cake marks batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

pub fn submit_site_rtt_batch(batch: &[shared_types::SiteRtt]) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::SiteRttBatch(batch.to_vec()),
            ))
            .is_err()
        {
            println!("Failed to send site RTT batch to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send site RTT batch to LTS2 client"
            ));
        }
    }
    Ok(()) // SUCCESS
}

#[allow(dead_code)]
pub fn get_lts_license_status() -> anyhow::Result<i32> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let (otx, orx) = oneshot::channel();
        if tx.send(LtsClientCommand::LicenseStatus(otx)).is_err() {
            error!("Failed to send license status request to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send license status request to LTS2 client"
            ));
        }
        return if let Ok(result) = orx.blocking_recv() {
            Ok(result)
        } else {
            error!("Failed to receive license status response from LTS2 client");
            Err(anyhow::anyhow!(
                "Failed to receive license status response from LTS2 client"
            ))
        };
    }
    Err(anyhow::anyhow!("Failed to get command channel"))
}

#[allow(dead_code)]
pub fn get_lts_license_trial_remaining() -> anyhow::Result<i32> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let (otx, orx) = oneshot::channel();
        if tx.send(LtsClientCommand::TrialDaysRemaining(otx)).is_err() {
            error!("Failed to send trial days remaining request to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send trial days remaining request to LTS2 client"
            ));
        }
        return if let Ok(result) = orx.blocking_recv() {
            Ok(result)
        } else {
            error!("Failed to receive trial days remaining response from LTS2 client");
            Err(anyhow::anyhow!(
                "Failed to receive trial days remaining response from LTS2 client"
            ))
        };
    }
    Err(anyhow::anyhow!("Failed to get command channel"))
}

pub async fn get_lts_license_status_async() -> anyhow::Result<i32> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let (otx, orx) = oneshot::channel();
        if tx.send(LtsClientCommand::LicenseStatus(otx)).is_err() {
            error!("Failed to send license status request to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send license status request to LTS2 client"
            ));
        }
        return if let Ok(result) = orx.await {
            Ok(result)
        } else {
            error!("Failed to receive license status response from LTS2 client");
            Err(anyhow::anyhow!(
                "Failed to receive license status response from LTS2 client"
            ))
        };
    }
    Err(anyhow::anyhow!("Failed to get command channel"))
}

pub async fn get_lts_license_trial_remaining_async() -> anyhow::Result<i32> {
    if let Ok(tx) = client_commands::get_command_channel() {
        let (otx, orx) = oneshot::channel();
        if tx.send(LtsClientCommand::TrialDaysRemaining(otx)).is_err() {
            error!("Failed to send trial days remaining request to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send trial days remaining request to LTS2 client"
            ));
        }
        return if let Ok(result) = orx.await {
            Ok(result)
        } else {
            error!("Failed to receive trial days remaining response from LTS2 client");
            Err(anyhow::anyhow!(
                "Failed to receive trial days remaining response from LTS2 client"
            ))
        };
    }
    Err(anyhow::anyhow!("Failed to get command channel"))
}

pub fn ingest_batch_complete() -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx.send(LtsClientCommand::IngestBatchComplete).is_err() {
            error!("Failed to send ingest batch complete message to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send ingest batch complete message to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn one_way_flow(
    start_time: u64,
    end_time: u64,
    local_ip: IpAddr,
    remote_ip: IpAddr,
    protocol: u8,
    dst_port: u16,
    src_port: u16,
    bytes: u64,
    circuit_hash: i64,
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::OneWayFlow {
                    start_time,
                    end_time,
                    local_ip,
                    remote_ip,
                    protocol,
                    dst_port,
                    src_port,
                    bytes,
                    circuit_hash,
                },
            ))
            .is_err()
        {
            error!("Failed to send one-way flow to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send one-way flow to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn two_way_flow(
    start_time: u64,
    end_time: u64,
    local_ip: IpAddr,
    remote_ip: IpAddr,
    protocol: u8,
    dst_port: u16,
    src_port: u16,
    bytes_down: u64,
    bytes_up: u64,
    packets_down: i64,
    packets_up: i64,
    retransmit_times_down: Vec<i64>,
    retransmit_times_up: Vec<i64>,
    rtt1: f32,
    rtt2: f32,
    circuit_hash: i64,
) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::TwoWayFlow {
                    start_time,
                    end_time,
                    local_ip,
                    remote_ip,
                    protocol,
                    dst_port,
                    src_port,
                    bytes_down,
                    bytes_up,
                    retransmit_times_down,
                    retransmit_times_up,
                    rtt1,
                    rtt2,
                    circuit_hash,
                    packets_down,
                    packets_up,
                },
            ))
            .is_err()
        {
            error!("Failed to send two-way flow to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send two-way flow to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn allow_subnet(ip_string: String) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::AllowSubnet(ip_string),
            ))
            .is_err()
        {
            error!("Failed to send allow subnet to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send allow subnet to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn ignore_subnet(ip_string: String) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::IgnoreSubnet(ip_string),
            ))
            .is_err()
        {
            error!("Failed to send ignore subnet to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send ignore subnet to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn submit_blackboard(bytes: &[u8]) -> anyhow::Result<()> {
    let bytes = bytes.to_vec();
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::BlackboardJson(bytes),
            ))
            .is_err()
        {
            error!("Failed to send blackboard JSON to LTS2 client");
            return Err(anyhow::anyhow!(
                "Failed to send blackboard JSON to LTS2 client"
            ));
        }
    }
    Ok(())
}

pub fn flow_count(timestamp: u64, flow_count: u64) -> anyhow::Result<()> {
    if let Ok(tx) = client_commands::get_command_channel() {
        if tx
            .send(LtsClientCommand::IngestData(
                ingestor::commands::IngestorCommand::FlowCount {
                    timestamp,
                    flow_count,
                },
            ))
            .is_err()
        {
            error!("Failed to send flow count to LTS2 client");
            return Err(anyhow::anyhow!("Failed to send flow count to LTS2 client"));
        }
    }
    Ok(())
}

// Command Interface

pub fn remote_command_count() -> u64 {
    remote_commands::count() as u64
}

pub fn get_commands(callback: fn(Vec<u8>)) {
    // Obtain a local list of commands, clearing the global list
    let commands_to_send = remote_commands::get();

    // Serialize the commands to CBOR
    let Ok(cbor) = serde_cbor::to_vec(&commands_to_send) else {
        warn!("Unable to deserialize remote commands.");
        return;
    };

    // Submit via the callback
    callback(cbor);

    // This ensures that the Vec is dropped and the Mutex is unlocked
}

pub fn get_remote_host() -> String {
    if let Ok(config) = load_config() {
        return config
            .long_term_stats
            .lts_url
            .clone()
            .unwrap_or("insight.libreqos.com".to_string());
    }
    "insight.libreqos.com".to_string()
}

pub fn get_node_id() -> String {
    if let Ok(config) = load_config() {
        return config.node_id.clone();
    }
    "UNKNOWN".to_string()
}
