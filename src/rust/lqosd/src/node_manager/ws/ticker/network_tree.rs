use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::BusRequest;
use lqos_config::NetworkJsonTransport;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub async fn network_tree(
    channels: Arc<PubSub>,
    _bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    // TODO: This function should be refactored to use the bus.
    if !channels.is_channel_alive(PublishedChannels::NetworkTree).await {
        return;
    }

    let data = {
        if let Ok(net_json) = NETWORK_JSON.read() {
            net_json
                .get_nodes_when_ready()
                .iter()
                .enumerate()
                .map(|(i, n)| (i, n.clone_to_transit()))
                .collect::<Vec<(usize, NetworkJsonTransport)>>()
        } else {
            Vec::new()
        }
    };

    let message = json!(
    {
        "event": PublishedChannels::NetworkTree.to_string(),
        "data": data,
    }
    ).to_string();
    channels.send(PublishedChannels::NetworkTree, message).await;
}

#[derive(Serialize)]
pub struct Circuit {
    pub ip: IpAddr,
    pub bytes_per_second: DownUpOrder<u64>,
    pub median_latency: Option<f32>,
    pub tcp_retransmits: DownUpOrder<u64>,
    pub circuit_id: Option<String>,
    pub device_id: Option<String>,
    pub parent_node: Option<String>,
    pub circuit_name: Option<String>,
    pub device_name: Option<String>,
    pub plan: DownUpOrder<u32>,
    pub last_seen_nanos: u64,
}

pub fn all_circuits() -> Vec<Circuit> {
    if let Ok(kernel_now) = time_since_boot() {
        if let Ok(devices) = SHAPED_DEVICES.read() {
            THROUGHPUT_TRACKER.
                raw_data.
                iter()
                .map(|v| {
                    let ip = v.key().as_ip();
                    let last_seen_nanos = if v.last_seen > 0 {
                        let last_seen_nanos = v.last_seen as u128;
                        let since_boot = Duration::from(kernel_now).as_nanos();
                        //println!("since_boot: {:?}, last_seen: {:?}", since_boot, last_seen_nanos);
                        (since_boot - last_seen_nanos) as u64
                    } else {
                        u64::MAX
                    };

                    // Map to circuit et al
                    let mut circuit_id = None;
                    let mut circuit_name = None;
                    let mut device_id = None;
                    let mut device_name = None;
                    let mut parent_node = None;
                    let mut plan = DownUpOrder::new(0, 0);
                    let lookup = match ip {
                        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                        IpAddr::V6(ip) => ip,
                    };
                    if let Some(c) = devices.trie.longest_match(lookup) {
                        circuit_id = Some(devices.devices[*c.1].circuit_id.clone());
                        circuit_name = Some(devices.devices[*c.1].circuit_name.clone());
                        device_id = Some(devices.devices[*c.1].device_id.clone());
                        device_name = Some(devices.devices[*c.1].device_name.clone());
                        parent_node = Some(devices.devices[*c.1].parent_node.clone());
                        plan.down = devices.devices[*c.1].download_max_mbps;
                        plan.up = devices.devices[*c.1].upload_max_mbps;
                    }

                    Circuit {
                        ip: v.key().as_ip(),
                        bytes_per_second: v.bytes_per_second,
                        median_latency: v.median_latency(),
                        tcp_retransmits: v.tcp_retransmits,
                        circuit_id,
                        device_id,
                        circuit_name,
                        device_name,
                        parent_node,
                        plan,
                        last_seen_nanos,
                    }
                }).collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

pub async fn all_subscribers(channels: Arc<PubSub>) {
    if !channels.is_channel_alive(PublishedChannels::NetworkTreeClients).await {
        return;
    }

    let devices = all_circuits();
    let message = json!(
        {
            "event": PublishedChannels::NetworkTreeClients.to_string(),
            "data": devices,
        }
        ).to_string();
    channels.send(PublishedChannels::NetworkTreeClients, message).await;
}