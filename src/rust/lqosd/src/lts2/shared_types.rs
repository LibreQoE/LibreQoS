//! Keep this synchronized with the server-side version.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

pub type ControlSender = std::sync::mpsc::Sender<LtsCommand>;
pub type ControlReceiver = std::sync::mpsc::Receiver<LtsCommand>;
pub type GetConfigFn = fn(&mut Lts2Config);
pub type SendStatusFn = fn(bool, i32, i32);
pub type StartLts2Fn = fn(GetConfigFn, SendStatusFn, ControlReceiver);

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct Lts2Config {
    /// The path to the root certificate for the LTS server
    pub path_to_certificate: Option<String>,
    /// The domain name of the LTS server
    pub domain: Option<String>,
    /// The license key for the LTS server
    pub license_key: Option<String>,
    /// The ID of the node
    pub node_id: String,
    /// The node name
    pub node_name: String,
}

#[repr(C)]
#[derive(Debug)]
pub enum LtsCommand {
    RequestFreeTrial(FreeTrialDetails, tokio::sync::oneshot::Sender<String>),
    RequestConnectionToExistingAccount {
        license_key: String,
        node_id: String,
        reply: tokio::sync::oneshot::Sender<String>,
    },
    TotalThroughput {
        timestamp: u64,
        download_bytes: u64,
        upload_bytes: u64,
        shaped_download_bytes: u64,
        shaped_upload_bytes: u64,
        packets_up: u64,
        packets_down: u64,
        max_rtt: Option<f32>,
        min_rtt: Option<f32>,
        median_rtt: Option<f32>,
        tcp_retransmits_down: i32,
        tcp_retransmits_up: i32,
        cake_marks_down: i32,
        cake_marks_up: i32,
        cake_drops_down: i32,
        cake_drops_up: i32,
    },
    ShapedDevices {
        timestamp: u64,
        devices: Vec<u8>,
    },
    CircuitThroughput {
        timestamp: u64,
        circuit_hash: i64,
        download_bytes: u64,
        upload_bytes: u64,
    },
    CircuitRetransmits {
        timestamp: u64,
        circuit_hash: i64,
        tcp_retransmits_down: i32,
        tcp_retransmits_up: i32,
    },
    CircuitRtt {
        timestamp: u64,
        circuit_hash: i64,
        median_rtt: f32,
    },
}

#[repr(C)]
#[derive(Debug, Clone, Deserialize)]
pub struct FreeTrialDetails {
    pub name: String,
    pub email: String,
    pub business_name: String,
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub country: String,
    pub phone: String,
    pub website: String,
}

#[repr(C)]
#[derive(Debug, Clone, Serialize)]
pub struct Lts2Circuit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub circuit_hash: i64,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
    pub parent_node: i64,
    pub devices: Vec<Lts2Device>,
}

#[repr(C)]
#[derive(Debug, Clone, Serialize)]
pub struct Lts2Device {
    pub device_id: String,
    pub device_name: String,
    pub device_hash: i64,
    pub mac: String,
    pub ipv4: Vec<([u8; 4], u8)>,
    pub ipv6: Vec<([u8; 16], u8)>,
    pub comment: String,
}