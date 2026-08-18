use crate::shaped_devices_tracker::circuit_live::{
    CircuitLiveRollup, CircuitLiveSnapshot, fresh_circuit_live_snapshot,
};
use lqos_config::ShapedDevice;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;

const DEFAULT_SHAPED_DEVICES_PAGE_SIZE: usize = 24;
const MAX_SHAPED_DEVICES_PAGE_SIZE: usize = 250;

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MaybeString {
        String(String),
        Null,
    }

    Ok(match Option::<MaybeString>::deserialize(deserializer)? {
        Some(MaybeString::String(value)) => Some(value),
        Some(MaybeString::Null) | None => None,
    })
}

/// Server-side paging and search query for the shaped-devices inventory page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShapedDevicesPageQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub search: Option<String>,
    /// Which inventory surface to display.
    #[serde(default)]
    pub kind: Option<ShapedDevicesPageKind>,
}

/// Which shaped-device inventory source to query.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShapedDevicesPageKind {
    Static,
    Dynamic,
}

/// A server-paged slice of shaped devices plus total result counts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShapedDevicesPage {
    pub query: ShapedDevicesPageQuery,
    pub total_rows: usize,
    pub total_circuits: usize,
    pub rows: Vec<ShapedDevice>,
}

fn normalized_page_size(query: &ShapedDevicesPageQuery) -> usize {
    query
        .page_size
        .unwrap_or(DEFAULT_SHAPED_DEVICES_PAGE_SIZE)
        .clamp(1, MAX_SHAPED_DEVICES_PAGE_SIZE)
}

fn alphabetic_inventory_order(left: &ShapedDevice, right: &ShapedDevice) -> Ordering {
    left.circuit_name
        .cmp(&right.circuit_name)
        .then_with(|| left.device_name.cmp(&right.device_name))
        .then_with(|| left.device_id.cmp(&right.device_id))
}

fn live_sort_rank(snapshot: &CircuitLiveSnapshot, device: &ShapedDevice) -> (u64, u64, u64, u64) {
    let live = snapshot.by_circuit_id.get(&device.circuit_id);
    let bytes_per_second = live
        .map(|row: &CircuitLiveRollup| row.bytes_per_second)
        .unwrap_or_default();
    let total_bytes_per_second = bytes_per_second.down.saturating_add(bytes_per_second.up);
    let last_seen_nanos = live.map(|row| row.last_seen_nanos).unwrap_or(u64::MAX);
    (
        total_bytes_per_second,
        bytes_per_second.down,
        bytes_per_second.up,
        last_seen_nanos,
    )
}

fn dynamic_inventory_order(
    snapshot: &CircuitLiveSnapshot,
    left: &ShapedDevice,
    right: &ShapedDevice,
) -> Ordering {
    let left_rank = live_sort_rank(snapshot, left);
    let right_rank = live_sort_rank(snapshot, right);
    right_rank
        .0
        .cmp(&left_rank.0)
        .then_with(|| right_rank.1.cmp(&left_rank.1))
        .then_with(|| right_rank.2.cmp(&left_rank.2))
        .then_with(|| left_rank.3.cmp(&right_rank.3))
        .then_with(|| alphabetic_inventory_order(left, right))
}

/// Returns one filtered, sorted page of shaped-device rows.
///
/// Static inventory rows are ordered alphabetically. Dynamic inventory rows are
/// ordered by current observed throughput before pagination so the busiest
/// circuits stay at the top of the page.
pub fn shaped_devices_page(query: ShapedDevicesPageQuery) -> ShapedDevicesPage {
    let page = query.page.unwrap_or(0);
    let page_size = normalized_page_size(&query);
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    let kind = query.kind.clone().unwrap_or(ShapedDevicesPageKind::Static);

    let matches_search =
        |device: &ShapedDevice| {
            if search.is_empty() {
                return true;
            }
            device.device_name.to_lowercase().contains(&search)
                || device.circuit_name.to_lowercase().contains(&search)
                || device.parent_node.to_lowercase().contains(&search)
                || device.circuit_id.to_lowercase().contains(&search)
                || device.device_id.to_lowercase().contains(&search)
                || device.mac.to_lowercase().contains(&search)
                || device.comment.to_lowercase().contains(&search)
                || device
                    .sqm_override
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&search)
                || device.ipv4.iter().any(|(addr, prefix)| {
                    format!("{addr}/{prefix}").to_lowercase().contains(&search)
                })
                || device.ipv6.iter().any(|(addr, prefix)| {
                    format!("{addr}/{prefix}").to_lowercase().contains(&search)
                })
        };

    let mut filtered: Vec<ShapedDevice> = match kind {
        ShapedDevicesPageKind::Static => lqos_network_devices::shaped_devices_catalog()
            .iter_devices()
            .filter(|device| matches_search(device))
            .cloned()
            .collect(),
        ShapedDevicesPageKind::Dynamic => lqos_network_devices::dynamic_circuits_snapshot()
            .iter()
            .map(|circuit| &circuit.shaped)
            .filter(|device| matches_search(device))
            .cloned()
            .collect(),
    };
    match kind {
        ShapedDevicesPageKind::Static => filtered.sort_by(alphabetic_inventory_order),
        ShapedDevicesPageKind::Dynamic => {
            let snapshot = fresh_circuit_live_snapshot();
            filtered.sort_by(|left, right| dynamic_inventory_order(snapshot.as_ref(), left, right));
        }
    }

    let total_rows = filtered.len();
    let total_circuits = filtered
        .iter()
        .map(|device| device.circuit_id.clone())
        .collect::<HashSet<_>>()
        .len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        let end = (start + page_size).min(total_rows);
        filtered[start..end].to_vec()
    };

    ShapedDevicesPage {
        query: ShapedDevicesPageQuery {
            page: Some(page),
            page_size: Some(page_size),
            search: if search.is_empty() {
                None
            } else {
                query.search
            },
            kind: Some(kind),
        },
        total_rows,
        total_circuits,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fxhash::FxHashMap;
    use lqos_utils::units::DownUpOrder;

    fn test_device(circuit_id: &str, circuit_name: &str, device_name: &str) -> ShapedDevice {
        ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_name.to_string(),
            device_id: format!("{circuit_id}-device"),
            device_name: device_name.to_string(),
            ..Default::default()
        }
    }

    fn test_snapshot(entries: &[(&str, u64, u64, u64)]) -> CircuitLiveSnapshot {
        let mut by_circuit_id = FxHashMap::default();
        for (circuit_id, down, up, last_seen_nanos) in entries {
            by_circuit_id.insert(
                (*circuit_id).to_string(),
                CircuitLiveRollup {
                    circuit_id: (*circuit_id).to_string(),
                    circuit_name: String::new(),
                    parent_node: String::new(),
                    device_names: Vec::new(),
                    ip_addrs: Vec::new(),
                    plan_mbps: DownUpOrder::default(),
                    bytes_per_second: DownUpOrder::new(*down, *up),
                    rtt_current_p50_nanos: DownUpOrder::default(),
                    qoo: DownUpOrder::default(),
                    tcp_retransmit_sample: DownUpOrder::default(),
                    last_seen_nanos: *last_seen_nanos,
                },
            );
        }
        CircuitLiveSnapshot { by_circuit_id }
    }

    #[test]
    fn dynamic_inventory_order_prefers_higher_combined_throughput() {
        let snapshot = test_snapshot(&[
            ("slow", 10, 20, 3_000),
            ("fast", 40, 90, 2_000),
            ("idle", 0, 0, u64::MAX),
        ]);
        let mut rows = [
            test_device("slow", "Slow Circuit", "Slow Device"),
            test_device("idle", "Idle Circuit", "Idle Device"),
            test_device("fast", "Fast Circuit", "Fast Device"),
        ];

        rows.sort_by(|left, right| dynamic_inventory_order(&snapshot, left, right));

        assert_eq!(
            rows.iter()
                .map(|row| row.circuit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["fast", "slow", "idle"]
        );
    }

    #[test]
    fn dynamic_inventory_order_uses_alphabetic_tie_breakers() {
        let snapshot = test_snapshot(&[("alpha", 25, 25, 100), ("beta", 25, 25, 100)]);
        let mut rows = [
            test_device("beta", "Beta Circuit", "Second Device"),
            test_device("alpha", "Alpha Circuit", "First Device"),
        ];

        rows.sort_by(|left, right| dynamic_inventory_order(&snapshot, left, right));

        assert_eq!(
            rows.iter()
                .map(|row| row.circuit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }
}
