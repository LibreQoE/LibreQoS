use crate::node_manager::ws::messages::{CircuitCapacityRow, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_utils::units::DownUpOrder;
use std::collections::HashMap;
use std::sync::Arc;

struct CircuitAccumulator {
    bytes: DownUpOrder<u64>,
    median_rtt: f32,
}

pub async fn circuit_capacity(channels: Arc<PubSub>) {
    if !channels
        .is_channel_alive(PublishedChannels::CircuitCapacity)
        .await
    {
        return;
    }

    let mut circuits: HashMap<String, CircuitAccumulator> = HashMap::new();

    // Aggregate the data by circuit id
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .for_each(|(_k, c)| {
            if let Some(circuit_id) = &c.circuit_id {
                if let Some(accumulator) = circuits.get_mut(circuit_id) {
                    accumulator.bytes += c.bytes_per_second;
                    if let Some(latency) = c.median_latency() {
                        accumulator.median_rtt = latency;
                    }
                } else {
                    circuits.insert(
                        circuit_id.clone(),
                        CircuitAccumulator {
                            bytes: c.bytes_per_second,
                            median_rtt: c.median_latency().unwrap_or(0.0),
                        },
                    );
                }
            }
        });

    // Map circuits to capacities
    let shaped_devices = SHAPED_DEVICES.load();
    let capacities: Vec<CircuitCapacityRow> = {
        circuits
            .iter()
            .filter_map(|(circuit_id, accumulator)| {
                if let Some(device) = shaped_devices
                    .devices
                    .iter()
                    .find(|sd| sd.circuit_id == *circuit_id)
                {
                    let down_mbps = (accumulator.bytes.down as f64 * 8.0) / 1_000_000.0;
                    let down = down_mbps / device.download_max_mbps as f64;
                    let up_mbps = (accumulator.bytes.up as f64 * 8.0) / 1_000_000.0;
                    let up = up_mbps / device.upload_max_mbps as f64;

                    Some(CircuitCapacityRow {
                        circuit_name: device.circuit_name.clone(),
                        circuit_id: circuit_id.clone(),
                        capacity: [down, up],
                        median_rtt: accumulator.median_rtt,
                    })
                } else {
                    None
                }
            })
            .collect()
    };

    let message = WsResponse::CircuitCapacity { data: capacities };
    channels
        .send(PublishedChannels::CircuitCapacity, message)
        .await;
}
