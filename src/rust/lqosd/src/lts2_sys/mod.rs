//! Provides an interface with Insight/LTS2

use parking_lot::Mutex;
use std::net::IpAddr;
pub(crate) mod lts2_client;
pub mod license_grant;
pub mod shared_types;

use crate::lts2_sys::shared_types::{FreeTrialDetails, LtsStatus};
use anyhow::Result;
use once_cell::sync::Lazy;
pub use shared_types::RemoteCommand;
pub mod control_channel;

pub fn start_lts2(
    control_tx: tokio::sync::mpsc::Sender<control_channel::ControlChannelCommand>,
) -> Result<()> {
    // Launch the process
    lts2_client::spawn_lts2(control_tx)?;

    Ok(())
}

pub fn request_free_trial(details: FreeTrialDetails) -> Result<String> {
    Ok(lts2_client::request_free_trial(details)?)
}

pub fn network_tree(timestamp: u64, tree: &[u8]) -> Result<()> {
    Ok(lts2_client::submit_network_tree(timestamp, tree)?)
}

pub fn shaped_devices(timestamp: u64, devices: &[u8]) -> Result<()> {
    Ok(lts2_client::submit_shaped_devices(timestamp, devices)?)
}

pub fn total_throughput(
    timestamp: u64,
    download_bytes: u64,
    upload_bytes: u64,
    shaped_download_bytes: u64,
    shaped_upload_bytes: u64,
    packets_down: u64,
    packets_up: u64,
    packets_tcp_down: u64,
    packets_tcp_up: u64,
    packets_udp_down: u64,
    packets_udp_up: u64,
    packets_icmp_down: u64,
    packets_icmp_up: u64,
    max_rtt: Option<f32>,
    min_rtt: Option<f32>,
    median_rtt: Option<f32>,
    tcp_retransmits_down: i32,
    tcp_retransmits_up: i32,
    cake_marks_down: i32,
    cake_marks_up: i32,
    cake_drops_down: i32,
    cake_drops_up: i32,
) -> Result<()> {
    Ok(lts2_client::submit_total_throughput(
        timestamp,
        download_bytes,
        upload_bytes,
        shaped_download_bytes,
        shaped_upload_bytes,
        packets_down,
        packets_up,
        packets_tcp_down,
        packets_tcp_up,
        packets_udp_down,
        packets_udp_up,
        packets_icmp_down,
        packets_icmp_up,
        max_rtt.is_some(),
        max_rtt.unwrap_or(0.0),
        min_rtt.is_some(),
        min_rtt.unwrap_or(0.0),
        median_rtt.is_some(),
        median_rtt.unwrap_or(0.0),
        tcp_retransmits_down,
        tcp_retransmits_up,
        cake_marks_down,
        cake_marks_up,
        cake_drops_down,
        cake_drops_up,
    )?)
}

pub fn shaper_utilization(
    tick: u64,
    average_cpu: f32,
    peak_cpu: f32,
    memory_percent: f32,
) -> Result<()> {
    Ok(lts2_client::submit_shaper_utilization(
        tick,
        average_cpu,
        peak_cpu,
        memory_percent,
    )?)
}

pub fn circuit_throughput(data: &[shared_types::CircuitThroughput]) -> Result<()> {
    Ok(lts2_client::submit_circuit_throughput_batch(data)?)
}

pub fn circuit_retransmits(data: &[shared_types::CircuitRetransmits]) -> Result<()> {
    Ok(lts2_client::submit_circuit_retransmits_batch(data)?)
}

pub fn circuit_rtt(data: &[shared_types::CircuitRtt]) -> Result<()> {
    Ok(lts2_client::submit_circuit_rtt_batch(data)?)
}

pub fn circuit_cake_drops(data: &[shared_types::CircuitCakeDrops]) -> Result<()> {
    Ok(lts2_client::submit_circuit_cake_drops_batch(data)?)
}

pub fn circuit_cake_marks(data: &[shared_types::CircuitCakeMarks]) -> Result<()> {
    Ok(lts2_client::submit_circuit_cake_marks_batch(data)?)
}

pub fn site_throughput(data: &[shared_types::SiteThroughput]) -> Result<()> {
    Ok(lts2_client::submit_site_throughput_batch(data)?)
}

pub fn site_retransmits(data: &[shared_types::SiteRetransmits]) -> Result<()> {
    Ok(lts2_client::submit_site_retransmits_batch(data)?)
}

pub fn site_rtt(data: &[shared_types::SiteRtt]) -> Result<()> {
    Ok(lts2_client::submit_site_rtt_batch(data)?)
}

pub fn site_cake_drops(data: &[shared_types::SiteCakeDrops]) -> Result<()> {
    Ok(lts2_client::submit_site_cake_drops_batch(data)?)
}

pub fn site_cake_marks(data: &[shared_types::SiteCakeMarks]) -> Result<()> {
    Ok(lts2_client::submit_site_cake_marks_batch(data)?)
}

pub fn get_lts_license_status() -> (LtsStatus, i32) {
    // Use async path under the hood to avoid blocking inside a Tokio runtime.
    // If we're already inside a runtime, switch to a blocking section and run a tiny
    // current-thread runtime there; otherwise, build a small runtime and block_on.
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build temporary Tokio runtime");
            rt.block_on(async {
                let remaining = lts2_client::get_lts_license_trial_remaining_async()
                    .await
                    .unwrap_or(0);
                let status = lts2_client::get_lts_license_status_async()
                    .await
                    .unwrap_or(-1);
                (LtsStatus::from_i32(status), remaining)
            })
        })
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build temporary Tokio runtime");
        rt.block_on(async {
            let remaining = lts2_client::get_lts_license_trial_remaining_async()
                .await
                .unwrap_or(0);
            let status = lts2_client::get_lts_license_status_async()
                .await
                .unwrap_or(-1);
            (LtsStatus::from_i32(status), remaining)
        })
    }
}

pub async fn get_lts_license_status_async() -> (LtsStatus, i32) {
    let remaining = lts2_client::get_lts_license_trial_remaining_async()
        .await
        .unwrap_or(0);
    let status = lts2_client::get_lts_license_status_async()
        .await
        .unwrap_or(-1);
    (LtsStatus::from_i32(status), remaining)
}

pub fn ingest_batch_complete() -> Result<()> {
    Ok(lts2_client::ingest_batch_complete()?)
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
    circuit_hash: Option<i64>,
) -> Result<()> {
    Ok(lts2_client::one_way_flow(
        start_time,
        end_time,
        local_ip,
        remote_ip,
        protocol,
        dst_port,
        src_port,
        bytes,
        circuit_hash.unwrap_or(0),
    )?)
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
    circuit_hash: Option<i64>,
) -> Result<()> {
    Ok(lts2_client::two_way_flow(
        start_time,
        end_time,
        local_ip,
        remote_ip,
        protocol,
        dst_port,
        src_port,
        bytes_down,
        bytes_up,
        packets_down,
        packets_up,
        retransmit_times_down,
        retransmit_times_up,
        rtt1,
        rtt2,
        circuit_hash.unwrap_or(0),
    )?)
}

pub fn ip_policies(allow_subnets: &Vec<String>, ignore_subnets: &Vec<String>) -> Result<()> {
    for subnet in allow_subnets {
        lts2_client::allow_subnet(subnet.to_string())?;
    }
    for subnet in ignore_subnets {
        lts2_client::ignore_subnet(subnet.to_string())?;
    }
    Ok(())
}

pub fn blackboard(json: &[u8]) -> Result<()> {
    lts2_client::submit_blackboard(json)?;
    Ok(())
}

pub fn flow_count(timestamp: u64, count: u64) -> Result<()> {
    Ok(lts2_client::flow_count(timestamp, count)?)
}

pub fn remote_command_count() -> u64 {
    lts2_client::remote_command_count()
}

fn command_callback(buffer: Vec<u8>) {
    let mut lock = COMMANDS.lock();
    lock.clear();
    let Ok(commands) = serde_cbor::from_slice::<Vec<shared_types::RemoteCommand>>(&buffer) else {
        return;
    };
    lock.extend(commands);
}

static COMMANDS: Lazy<Mutex<Vec<shared_types::RemoteCommand>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

pub fn remote_commands() -> Vec<shared_types::RemoteCommand> {
    lts2_client::get_commands(command_callback);

    let lock = COMMANDS.lock();
    lock.clone()
}
