use anyhow::Result;
use lqos_bus::{BusResponse, IpMapping, TcHandle};
use lqos_sys::XdpIpAddress;

fn expect_ack(result: Result<()>) -> BusResponse {
    if result.is_ok() {
        BusResponse::Ack
    } else {
        BusResponse::Fail(format!("{:?}", result))
    }
}

pub(crate) fn map_ip_to_flow(
    ip_address: &str,
    tc_handle: &TcHandle,
    cpu: u32,
    upload: bool,
) -> BusResponse {
    expect_ack(lqos_sys::add_ip_to_tc(
        &ip_address,
        *tc_handle,
        cpu,
        upload,
    ))
}

pub(crate) fn del_ip_flow(ip_address: &str, upload: bool) -> BusResponse {
    expect_ack(lqos_sys::del_ip_from_tc(ip_address, upload))
}

pub(crate) fn clear_ip_flows() -> BusResponse {
    expect_ack(lqos_sys::clear_ips_from_tc())
}

pub(crate) fn list_mapped_ips() -> BusResponse {
    if let Ok(raw) = lqos_sys::list_mapped_ips() {
        let data = raw
            .iter()
            .map(|(ip_key, ip_data)| IpMapping {
                ip_address: XdpIpAddress(ip_key.address).as_ip().to_string(),
                prefix_length: ip_key.prefixlen,
                tc_handle: TcHandle::from_u32(ip_data.tc_handle),
                cpu: ip_data.cpu,
            })
            .collect();
        BusResponse::MappedIps(data)
    } else {
        BusResponse::Fail("Unable to get IP map".to_string())
    }
}
