use lqos_config::ShapedDevice;
use serde::{Deserialize, Deserializer, Serialize};
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

/// Returns one filtered, sorted page of shaped-device rows.
pub fn shaped_devices_page(query: ShapedDevicesPageQuery) -> ShapedDevicesPage {
    let page = query.page.unwrap_or(0);
    let page_size = normalized_page_size(&query);
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    let kind = query.kind.clone().unwrap_or(ShapedDevicesPageKind::Static);

    let matches_search = |device: &ShapedDevice| {
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
            || device
                .ipv4
                .iter()
                .any(|(addr, prefix)| format!("{addr}/{prefix}").to_lowercase().contains(&search))
            || device
                .ipv6
                .iter()
                .any(|(addr, prefix)| format!("{addr}/{prefix}").to_lowercase().contains(&search))
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
    filtered.sort_by(|left, right| {
        left.circuit_name
            .cmp(&right.circuit_name)
            .then_with(|| left.device_name.cmp(&right.device_name))
            .then_with(|| left.device_id.cmp(&right.device_id))
    });

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
