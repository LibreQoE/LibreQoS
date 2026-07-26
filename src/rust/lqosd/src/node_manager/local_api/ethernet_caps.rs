use lqos_config::{
    CircuitEthernetMetadata, CircuitEthernetMetadataFile, EthernetCapTargetKind, load_config,
};
use lqos_utils::normalize_circuit_id_key;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

const DEFAULT_ETHERNET_CAPS_PAGE_SIZE: usize = 100;
const MAX_ETHERNET_CAPS_PAGE_SIZE: usize = 250;

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

/// Normalized Ethernet-cap tier used by UI badges and filters.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EthernetCapTier {
    TenM,
    HundredM,
    GigPlus,
}

impl EthernetCapTier {
    /// Returns the short operator-facing label for this Ethernet tier.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::TenM => "10M",
            Self::HundredM => "100M",
            Self::GigPlus => "1G",
        }
    }
}

/// Compact Ethernet-cap badge payload for circuit tables and detail views.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EthernetCapBadge {
    /// Normalized tier used for styling and filtering.
    pub tier: EthernetCapTier,
    /// Short display label such as `10M`, `100M`, or `1G`.
    pub tier_label: String,
    /// Negotiated Ethernet speed in Mbps reported by the limiting interface.
    pub negotiated_ethernet_mbps: u64,
    /// Requested download max before the cap was applied.
    pub requested_download_mbps: f32,
    /// Requested upload max before the cap was applied.
    pub requested_upload_mbps: f32,
    /// Applied download max after the cap.
    pub applied_download_mbps: f32,
    /// Applied upload max after the cap.
    pub applied_upload_mbps: f32,
}

/// Server-side paging/filter query for the Ethernet caps review page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EthernetCapsPageQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub search: Option<String>,
    pub tier: Option<EthernetCapTier>,
}

/// One Ethernet-limited circuit or topology-node row for the review page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EthernetCapsPageRow {
    /// Stable circuit identifier retained for clients that only understand circuit rows.
    pub circuit_id: String,
    /// Human-facing circuit name retained for clients that only understand circuit rows.
    pub circuit_name: String,
    /// Whether this row represents a circuit or topology node.
    pub target_kind: EthernetCapTargetKind,
    /// Stable target identifier used for deep-linking.
    pub target_id: String,
    /// Human-facing target name.
    pub target_name: String,
    /// Parent node from shaped devices when available.
    pub parent_node: String,
    /// Compact warning badge metadata.
    pub badge: EthernetCapBadge,
    /// Device name that supplied the limiting Ethernet speed when known.
    pub limiting_device_name: Option<String>,
    /// Interface name that supplied the limiting Ethernet speed when known.
    pub limiting_interface_name: Option<String>,
}

/// A server-paged slice of Ethernet-capped circuits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EthernetCapsPage {
    /// The normalized query that produced this page.
    pub query: EthernetCapsPageQuery,
    /// Total rows matching the query before paging.
    pub total_rows: usize,
    /// Page rows after filtering and paging.
    pub rows: Vec<EthernetCapsPageRow>,
}

fn normalized_page_size(query: &EthernetCapsPageQuery) -> usize {
    query
        .page_size
        .unwrap_or(DEFAULT_ETHERNET_CAPS_PAGE_SIZE)
        .clamp(1, MAX_ETHERNET_CAPS_PAGE_SIZE)
}

fn tier_for_speed(negotiated_ethernet_mbps: u64) -> EthernetCapTier {
    match negotiated_ethernet_mbps {
        0..=10 => EthernetCapTier::TenM,
        11..=100 => EthernetCapTier::HundredM,
        _ => EthernetCapTier::GigPlus,
    }
}

fn advisory_to_badge(advisory: &CircuitEthernetMetadata) -> Option<EthernetCapBadge> {
    if !advisory.auto_capped {
        return None;
    }

    let tier = tier_for_speed(advisory.negotiated_ethernet_mbps);
    Some(EthernetCapBadge {
        tier: tier.clone(),
        tier_label: tier.label().to_string(),
        negotiated_ethernet_mbps: advisory.negotiated_ethernet_mbps,
        requested_download_mbps: advisory.requested_download_mbps,
        requested_upload_mbps: advisory.requested_upload_mbps,
        applied_download_mbps: advisory.applied_download_mbps,
        applied_upload_mbps: advisory.applied_upload_mbps,
    })
}

fn load_advisory_file() -> Option<CircuitEthernetMetadataFile> {
    let cfg = load_config().ok()?;
    let path = lqos_config::circuit_ethernet_metadata_path(cfg.as_ref());
    let payload = std::fs::read(path).ok()?;
    serde_json::from_slice(&payload).ok()
}

fn parent_node_by_circuit_id() -> HashMap<String, String> {
    let catalog = lqos_network_devices::shaped_devices_catalog();
    let mut parent_nodes = HashMap::new();
    for device in catalog.iter_devices() {
        let circuit_id = device.circuit_id.trim();
        if circuit_id.is_empty() {
            continue;
        }
        parent_nodes
            .entry(circuit_id.to_string())
            .or_insert_with(|| device.parent_node.trim().to_string());
    }
    parent_nodes
}

fn sort_rank(tier: &EthernetCapTier) -> u8 {
    match tier {
        EthernetCapTier::TenM => 0,
        EthernetCapTier::HundredM => 1,
        EthernetCapTier::GigPlus => 2,
    }
}

fn advisory_to_page_row(
    advisory: &CircuitEthernetMetadata,
    parent_nodes: &HashMap<String, String>,
) -> Option<EthernetCapsPageRow> {
    let badge = advisory_to_badge(advisory)?;
    let target_id = advisory.target_id_or_circuit_id().to_string();
    let target_name = advisory.target_name_or_circuit_name().to_string();
    let parent_node = if advisory.target_kind == EthernetCapTargetKind::Circuit {
        parent_nodes
            .get(&advisory.circuit_id)
            .cloned()
            .unwrap_or_default()
    } else {
        String::new()
    };

    Some(EthernetCapsPageRow {
        circuit_id: advisory.circuit_id.clone(),
        circuit_name: advisory.circuit_name.clone(),
        target_kind: advisory.target_kind.clone(),
        target_id,
        target_name,
        parent_node,
        badge,
        limiting_device_name: advisory.limiting_device_name.clone(),
        limiting_interface_name: advisory.limiting_interface_name.clone(),
    })
}

/// Returns a compact badge map keyed by circuit ID for Ethernet auto-capped circuits.
pub(crate) fn ethernet_cap_badge_map() -> HashMap<String, EthernetCapBadge> {
    let mut badges = HashMap::new();
    let Some(file) = load_advisory_file() else {
        return badges;
    };
    for advisory in file.circuits {
        if advisory.target_kind != EthernetCapTargetKind::Circuit {
            continue;
        }
        let Some(badge) = advisory_to_badge(&advisory) else {
            continue;
        };
        badges.insert(normalize_circuit_id_key(&advisory.circuit_id), badge);
    }
    badges
}

/// Finds one Ethernet advisory for the requested circuit and matching shaped-device IDs.
pub(crate) fn ethernet_advisory_for_circuit(
    circuit_id: &str,
    device_ids: &std::collections::HashSet<&str>,
) -> Option<CircuitEthernetMetadata> {
    let file = load_advisory_file()?;
    file.circuits.into_iter().find(|entry| {
        entry.target_kind == EthernetCapTargetKind::Circuit
            && entry.auto_capped
            && entry.circuit_id.eq_ignore_ascii_case(circuit_id)
            && entry
                .device_ids
                .iter()
                .any(|device_id| device_ids.contains(device_id.as_str()))
    })
}

/// Returns one filtered, sorted page of Ethernet-limited circuits for operator review.
pub fn ethernet_caps_page(query: EthernetCapsPageQuery) -> EthernetCapsPage {
    let page = query.page.unwrap_or(0);
    let page_size = normalized_page_size(&query);
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    let parent_nodes = parent_node_by_circuit_id();

    let mut rows = Vec::new();
    if let Some(file) = load_advisory_file() {
        for advisory in file.circuits {
            let Some(row) = advisory_to_page_row(&advisory, &parent_nodes) else {
                continue;
            };
            if let Some(filter_tier) = query.tier.as_ref()
                && &row.badge.tier != filter_tier
            {
                continue;
            }
            if !search.is_empty() {
                let limiting_device = row.limiting_device_name.as_deref().unwrap_or("");
                let limiting_interface = row.limiting_interface_name.as_deref().unwrap_or("");
                if !row.target_id.to_lowercase().contains(&search)
                    && !row.target_name.to_lowercase().contains(&search)
                    && !row.parent_node.to_lowercase().contains(&search)
                    && !row.badge.tier_label.to_lowercase().contains(&search)
                    && !limiting_device.to_lowercase().contains(&search)
                    && !limiting_interface.to_lowercase().contains(&search)
                {
                    continue;
                }
            }
            rows.push(row);
        }
    }

    rows.sort_by(|left, right| {
        sort_rank(&left.badge.tier)
            .cmp(&sort_rank(&right.badge.tier))
            .then_with(|| left.target_name.cmp(&right.target_name))
            .then_with(|| left.target_id.cmp(&right.target_id))
    });

    let total_rows = rows.len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        let end = (start + page_size).min(total_rows);
        rows[start..end].to_vec()
    };

    EthernetCapsPage {
        query: EthernetCapsPageQuery {
            page: Some(page),
            page_size: Some(page_size),
            search: if search.is_empty() {
                None
            } else {
                query.search
            },
            tier: query.tier,
        },
        total_rows,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::{EthernetCapTargetKind, EthernetCapTier, advisory_to_page_row, sort_rank, tier_for_speed};
    use lqos_config::CircuitEthernetMetadata;
    use std::collections::HashMap;

    #[test]
    fn ethernet_cap_tier_classifies_expected_speeds() {
        assert_eq!(tier_for_speed(10), EthernetCapTier::TenM);
        assert_eq!(tier_for_speed(100), EthernetCapTier::HundredM);
        assert_eq!(tier_for_speed(1000), EthernetCapTier::GigPlus);
    }

    #[test]
    fn ethernet_cap_sort_rank_prioritizes_low_speed_tiers() {
        assert!(sort_rank(&EthernetCapTier::TenM) < sort_rank(&EthernetCapTier::HundredM));
        assert!(sort_rank(&EthernetCapTier::HundredM) < sort_rank(&EthernetCapTier::GigPlus));
    }

    #[test]
    fn topology_node_advisory_becomes_a_node_page_row() {
        let advisory = CircuitEthernetMetadata {
            target_kind: EthernetCapTargetKind::Node,
            target_id: "uisp:device:ap-1".to_string(),
            target_name: "AP One".to_string(),
            circuit_id: "uisp:device:ap-1".to_string(),
            circuit_name: "AP One".to_string(),
            negotiated_ethernet_mbps: 100,
            requested_download_mbps: 500.0,
            requested_upload_mbps: 200.0,
            applied_download_mbps: 94.0,
            applied_upload_mbps: 94.0,
            auto_capped: true,
            ..Default::default()
        };

        let row = advisory_to_page_row(&advisory, &HashMap::new())
            .expect("auto-capped topology node should appear on the page");

        assert_eq!(row.target_kind, EthernetCapTargetKind::Node);
        assert_eq!(row.target_id, "uisp:device:ap-1");
        assert_eq!(row.target_name, "AP One");
        assert!(row.parent_node.is_empty());
    }
}
