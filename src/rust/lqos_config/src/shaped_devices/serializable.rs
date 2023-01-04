
use std::net::{Ipv4Addr, Ipv6Addr};
use serde::Serialize;
use crate::ShapedDevice;

// Example: StringRecord(["1", "968 Circle St., Gurnee, IL 60031", "1", "Device 1", "", "", "192.168.101.2", "", "25", "5", "10000", "10000", ""])
#[derive(Serialize)]
pub(crate) struct SerializableShapedDevice {
    pub circuit_id: String,
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub ipv4: String,
    pub ipv6: String,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
    pub comment: String,
}

impl From<&ShapedDevice> for SerializableShapedDevice {
    fn from(d: &ShapedDevice) -> Self {
        Self {
            circuit_id: d.circuit_id.clone(),
            circuit_name: d.circuit_name.clone(),
            device_id: d.device_id.clone(),
            device_name: d.device_name.clone(),
            parent_node: d.parent_node.clone(),
            mac: d.mac.clone(),
            ipv4: ipv4_list_to_string(&d.ipv4),
            ipv6: ipv6_list_to_string(&d.ipv6),
            download_min_mbps: d.download_min_mbps,
            upload_min_mbps: d.upload_min_mbps,
            download_max_mbps: d.download_max_mbps,
            upload_max_mbps: d.upload_max_mbps,
            comment: d.comment.clone()
        }
    }
}

fn ipv4_to_string(ip: &(Ipv4Addr, u32)) -> String {
    if ip.1 == 32 {
        format!("{}", ip.0)
    } else {
        format!{"{}/{}", ip.0, ip.1}
    }
}

fn ipv4_list_to_string(ips: &[(Ipv4Addr, u32)]) -> String {
    if ips.len() == 0 {
        return String::new();
    }
    if ips.len() == 1 {
        return ipv4_to_string(&ips[0]);
    }
    let mut buffer = String::new();
    for i in 0..ips.len()-1 {
        buffer += &format!("{}, ", ipv4_to_string(&ips[i]));
    }
    buffer += &ipv4_to_string(&ips[ips.len()-1]);
    String::new()
}

fn ipv6_to_string(ip: &(Ipv6Addr, u32)) -> String {
    if ip.1 == 32 {
        format!("{}", ip.0)
    } else {
        format!{"{}/{}", ip.0, ip.1}
    }
}

fn ipv6_list_to_string(ips: &[(Ipv6Addr, u32)]) -> String {
    if ips.len() == 0 {
        return String::new();
    }
    if ips.len() == 1 {
        return ipv6_to_string(&ips[0]);
    }
    let mut buffer = String::new();
    for i in 0..ips.len()-1 {
        buffer += &format!("{}, ", ipv6_to_string(&ips[i]));
    }
    buffer += &ipv6_to_string(&ips[ips.len()-1]);
    String::new()
}