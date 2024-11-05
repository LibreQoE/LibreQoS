use serde::Deserialize;

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
#[derive(Debug, Clone, Copy)]
pub struct CircuitThroughput {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub packets_down: u64,
    pub packets_up: u64,
    pub packets_tcp_down: u64,
    pub packets_tcp_up: u64,
    pub packets_udp_down: u64,
    pub packets_udp_up: u64,
    pub packets_icmp_down: u64,
    pub packets_icmp_up: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CircuitRetransmits {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub tcp_retransmits_down: i32,
    pub tcp_retransmits_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CircuitRtt {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub median_rtt: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CircuitCakeDrops {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub cake_drops_down: i32,
    pub cake_drops_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CircuitCakeMarks {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub cake_marks_down: i32,
    pub cake_marks_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SiteThroughput {
    pub timestamp: u64,
    pub site_hash: i64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub packets_down: u64,
    pub packets_up: u64,
    pub packets_tcp_down: u64,
    pub packets_tcp_up: u64,
    pub packets_udp_down: u64,
    pub packets_udp_up: u64,
    pub packets_icmp_down: u64,
    pub packets_icmp_up: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SiteRetransmits {
    pub timestamp: u64,
    pub site_hash: i64,
    pub tcp_retransmits_down: i32,
    pub tcp_retransmits_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SiteCakeDrops {
    pub timestamp: u64,
    pub site_hash: i64,
    pub cake_drops_down: i32,
    pub cake_drops_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SiteCakeMarks {
    pub timestamp: u64,
    pub site_hash: i64,
    pub cake_marks_down: i32,
    pub cake_marks_up: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SiteRtt {
    pub timestamp: u64,
    pub site_hash: i64,
    pub median_rtt: f32,
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
#[repr(i32)]
pub enum LtsStatus {
    NotChecked = -1,
    AlwaysFree = 0,
    FreeTrial = 1,
    SelfHosted = 2,
    ApiOnly = 3,
    Full = 4,
    Invalid = 5,
}

impl LtsStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            -1 => LtsStatus::NotChecked,
            1 => LtsStatus::AlwaysFree,
            2 => LtsStatus::FreeTrial,
            3 => LtsStatus::SelfHosted,
            4 => LtsStatus::ApiOnly,
            5 => LtsStatus::Full,
            _ => LtsStatus::Invalid,
        }
    }
}