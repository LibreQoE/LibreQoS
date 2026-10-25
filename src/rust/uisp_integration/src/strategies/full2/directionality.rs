use crate::uisp_types::{UispDevice, UispSite};
use lqos_config::Config;
use std::collections::{HashMap, HashSet};
use uisp::Device;

/// Site latitude differences below this are treated as east/west (no
/// north/south winner). ~1e-5 degrees is about one metre, so ordinary
/// geocoding noise does not flip a link.
const LATITUDE_EPSILON: f32 = 1e-5;

#[derive(Debug, Clone)]
struct SiteTopology {
    latitude: Option<f32>,
    parent_id: Option<String>,
}

/// The UISP site assignment is the orientation signal we trust first.
#[derive(Debug, Clone, Default)]
pub(crate) struct LinkTopology {
    site_by_id: HashMap<String, SiteTopology>,
    site_id_by_device_id: HashMap<String, String>,
}

impl LinkTopology {
    pub(crate) fn from_uisp_data(sites: &[UispSite], devices: &[UispDevice]) -> Self {
        let site_by_id = sites
            .iter()
            .map(|site| {
                (
                    site.id.clone(),
                    SiteTopology {
                        latitude: site.latitude,
                        parent_id: site.uisp_parent_id.clone(),
                    },
                )
            })
            .collect();
        let site_id_by_device_id = devices
            .iter()
            .filter(|device| !device.site_id.is_empty())
            .map(|device| (device.id.clone(), device.site_id.clone()))
            .collect();
        Self {
            site_by_id,
            site_id_by_device_id,
        }
    }

    /// Topology-first orientation:
    /// 1. parent/upstream site wins;
    /// 2. otherwise the northern site wins when there is a real latitude delta;
    /// 3. otherwise None, letting the label-based fallback decide.
    fn orientation(&self, id_a: &str, id_b: &str) -> Option<LinkOrientation> {
        let site_a = self.site_id_by_device_id.get(id_a)?;
        let site_b = self.site_id_by_device_id.get(id_b)?;
        if site_a == site_b {
            return None;
        }
        if self.is_ancestor(site_a, site_b) {
            return Some(LinkOrientation::AIsAp);
        }
        if self.is_ancestor(site_b, site_a) {
            return Some(LinkOrientation::BIsAp);
        }

        let lat_a = self.site_by_id.get(site_a)?.latitude?;
        let lat_b = self.site_by_id.get(site_b)?.latitude?;
        if (lat_a - lat_b).abs() <= LATITUDE_EPSILON {
            return None;
        }
        if lat_a > lat_b {
            Some(LinkOrientation::AIsAp)
        } else {
            Some(LinkOrientation::BIsAp)
        }
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let mut current = descendant;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.to_string()) {
                return false;
            }
            let Some(site) = self.site_by_id.get(current) else {
                return false;
            };
            let Some(parent) = site.parent_id.as_deref() else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
    }

    #[cfg(test)]
    fn from_test_sites(
        sites: &[(&str, Option<f32>, Option<&str>)],
        device_sites: &[(&str, &str)],
    ) -> Self {
        Self {
            site_by_id: sites
                .iter()
                .map(|(id, latitude, parent_id)| {
                    (
                        (*id).to_string(),
                        SiteTopology {
                            latitude: *latitude,
                            parent_id: parent_id.map(str::to_string),
                        },
                    )
                })
                .collect(),
            site_id_by_device_id: device_sites
                .iter()
                .map(|(device, site)| ((*device).to_string(), (*site).to_string()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceLinkMeta {
    pub(crate) ap_device_id: Option<String>,
    pub(crate) role: Option<String>,
    pub(crate) wireless_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkOrientation {
    AIsAp,
    BIsAp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RadioRole {
    Ap,
    Station,
}

pub(crate) fn build_device_link_meta_map(
    devices_raw: &[Device],
) -> HashMap<String, DeviceLinkMeta> {
    devices_raw
        .iter()
        .map(|d| {
            let ap_device_id = d
                .attributes
                .as_ref()
                .and_then(|a| a.apDevice.as_ref())
                .and_then(|ap| ap.id.clone());
            let role = d.identification.role.clone();
            let wireless_mode = d.overview.as_ref().and_then(|o| o.wirelessMode.clone());
            (
                d.identification.id.clone(),
                DeviceLinkMeta {
                    ap_device_id,
                    role,
                    wireless_mode,
                },
            )
        })
        .collect()
}

pub(crate) fn build_device_capacity_map(devices: &[UispDevice]) -> HashMap<String, (u64, u64)> {
    devices
        .iter()
        .map(|d| (d.id.clone(), (d.download, d.upload)))
        .collect()
}

fn role_kind(role: Option<&str>) -> Option<RadioRole> {
    let role = role?.trim().to_ascii_lowercase();
    match role.as_str() {
        "ap" => Some(RadioRole::Ap),
        "station" | "sta" | "cpe" => Some(RadioRole::Station),
        _ => None,
    }
}

fn wireless_mode_kind(mode: Option<&str>) -> Option<RadioRole> {
    let mode = mode?.trim().to_ascii_lowercase();
    if mode.starts_with("ap") {
        return Some(RadioRole::Ap);
    }
    if mode.starts_with("sta") || mode.starts_with("station") {
        return Some(RadioRole::Station);
    }
    None
}

pub(crate) fn classify_ap_station(
    meta_by_id: &HashMap<String, DeviceLinkMeta>,
    id_a: &str,
    id_b: &str,
) -> Option<LinkOrientation> {
    classify_ap_station_with_topology(meta_by_id, None, id_a, id_b)
}

pub(crate) fn classify_ap_station_with_topology(
    meta_by_id: &HashMap<String, DeviceLinkMeta>,
    topology: Option<&LinkTopology>,
    id_a: &str,
    id_b: &str,
) -> Option<LinkOrientation> {
    if let Some(topology) = topology
        && let Some(orientation) = topology.orientation(id_a, id_b)
    {
        return Some(orientation);
    }

    let meta_a = meta_by_id.get(id_a)?;
    let meta_b = meta_by_id.get(id_b)?;

    // UISP (devices?withInterfaces=true) also fills `attributes.apDevice` on the AP
    // end of PtP/bridge links, pointing back at the station, so a mutual reference
    // is not evidence of orientation on its own. Use the reference only when one
    // end is positively labelled: trust a positively-labelled AP end, or a
    // positively-labelled station end (the other end is then the AP). If both are
    // AP, both are station, or neither is labelled, decline instead of making the
    // answer depend on argument order.
    let a_refs_b = meta_a.ap_device_id.as_deref() == Some(id_b);
    let b_refs_a = meta_b.ap_device_id.as_deref() == Some(id_a);
    let a_is_ap = role_kind(meta_a.role.as_deref()) == Some(RadioRole::Ap)
        || wireless_mode_kind(meta_a.wireless_mode.as_deref()) == Some(RadioRole::Ap);
    let b_is_ap = role_kind(meta_b.role.as_deref()) == Some(RadioRole::Ap)
        || wireless_mode_kind(meta_b.wireless_mode.as_deref()) == Some(RadioRole::Ap);
    let a_is_station = role_kind(meta_a.role.as_deref()) == Some(RadioRole::Station)
        || wireless_mode_kind(meta_a.wireless_mode.as_deref()) == Some(RadioRole::Station);
    let b_is_station = role_kind(meta_b.role.as_deref()) == Some(RadioRole::Station)
        || wireless_mode_kind(meta_b.wireless_mode.as_deref()) == Some(RadioRole::Station);
    if a_refs_b && b_refs_a {
        if a_is_ap && !b_is_ap {
            return Some(LinkOrientation::AIsAp);
        }
        if b_is_ap && !a_is_ap {
            return Some(LinkOrientation::BIsAp);
        }
        if a_is_ap && b_is_ap {
            return None;
        }
        if a_is_station && !b_is_station {
            return Some(LinkOrientation::BIsAp);
        }
        if b_is_station && !a_is_station {
            return Some(LinkOrientation::AIsAp);
        }
        return None;
    }
    // A one-way reference keeps the historical behavior: the referencing end is
    // the station unless it is itself positively an AP.
    if a_refs_b && !a_is_ap {
        return Some(LinkOrientation::BIsAp);
    }
    if b_refs_a && !b_is_ap {
        return Some(LinkOrientation::AIsAp);
    }

    // Next: explicit role field.
    match (
        role_kind(meta_a.role.as_deref()),
        role_kind(meta_b.role.as_deref()),
    ) {
        (Some(RadioRole::Ap), Some(RadioRole::Station)) => return Some(LinkOrientation::AIsAp),
        (Some(RadioRole::Station), Some(RadioRole::Ap)) => return Some(LinkOrientation::BIsAp),
        _ => {}
    }

    // Last: infer from wirelessMode prefix (ap*/sta*).
    match (
        wireless_mode_kind(meta_a.wireless_mode.as_deref()),
        wireless_mode_kind(meta_b.wireless_mode.as_deref()),
    ) {
        (Some(RadioRole::Ap), Some(RadioRole::Station)) => Some(LinkOrientation::AIsAp),
        (Some(RadioRole::Station), Some(RadioRole::Ap)) => Some(LinkOrientation::BIsAp),
        _ => None,
    }
}

pub(crate) fn directed_caps_mbps(
    meta_by_id: &HashMap<String, DeviceLinkMeta>,
    caps_by_id: &HashMap<String, (u64, u64)>,
    config: &Config,
    id_a: &str,
    id_b: &str,
) -> Option<(u64, u64)> {
    directed_caps_mbps_with_topology(meta_by_id, None, caps_by_id, config, id_a, id_b)
}

pub(crate) fn directed_caps_mbps_with_topology(
    meta_by_id: &HashMap<String, DeviceLinkMeta>,
    topology: Option<&LinkTopology>,
    caps_by_id: &HashMap<String, (u64, u64)>,
    config: &Config,
    id_a: &str,
    id_b: &str,
) -> Option<(u64, u64)> {
    let orientation = classify_ap_station_with_topology(meta_by_id, topology, id_a, id_b)?;

    let (a_down, a_up) = caps_by_id.get(id_a).copied().unwrap_or((
        config.queues.generated_pn_download_mbps,
        config.queues.generated_pn_upload_mbps,
    ));
    let (b_down, b_up) = caps_by_id.get(id_b).copied().unwrap_or((
        config.queues.generated_pn_download_mbps,
        config.queues.generated_pn_upload_mbps,
    ));

    let (ap_down, ap_up, sta_down, sta_up) = match orientation {
        LinkOrientation::AIsAp => (a_down, a_up, b_down, b_up),
        LinkOrientation::BIsAp => (b_down, b_up, a_down, a_up),
    };

    // UISP can disagree between ends. Use the conservative (minimum) capacity per direction.
    let mut down_mbps = ap_down.min(sta_down);
    let mut up_mbps = ap_up.min(sta_up);

    if down_mbps < 1 {
        down_mbps = config.queues.generated_pn_download_mbps;
    }
    if up_mbps < 1 {
        up_mbps = config.queues.generated_pn_upload_mbps;
    }

    match orientation {
        // A -> B is AP -> station (download direction on the link).
        LinkOrientation::AIsAp => Some((down_mbps, up_mbps)),
        // A -> B is station -> AP (upload direction on the link).
        LinkOrientation::BIsAp => Some((up_mbps, down_mbps)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_raw_device(
        id: &str,
        role: Option<&str>,
        wireless_mode: Option<&str>,
        ap_device_id: Option<&str>,
    ) -> Device {
        let mut obj = json!({
            "identification": {
                "id": id,
                "hostname": id,
                "role": role,
            }
        });

        if let Some(mode) = wireless_mode {
            obj["overview"] = json!({ "wirelessMode": mode });
        }
        if let Some(ap_id) = ap_device_id {
            obj["attributes"] = json!({ "apDevice": { "id": ap_id } });
        }

        serde_json::from_value(obj).expect("device JSON must deserialize")
    }

    fn caps_map(entries: &[(&str, u64, u64)]) -> HashMap<String, (u64, u64)> {
        entries
            .iter()
            .map(|(id, down, up)| (id.to_string(), (*down, *up)))
            .collect()
    }

    #[test]
    fn attributes_based_orientation_is_stable() {
        let ap = mk_raw_device("ap", None, None, None);
        let sta = mk_raw_device("sta", None, None, Some("ap"));
        let meta = build_device_link_meta_map(&[ap, sta]);

        let mut cfg = Config::default();
        cfg.queues.generated_pn_download_mbps = 999;
        cfg.queues.generated_pn_upload_mbps = 888;

        let caps = caps_map(&[("ap", 200, 50), ("sta", 180, 60)]);

        // A=sta, B=ap => sta->ap is upload direction, ap->sta is download direction.
        let (cap_ab, cap_ba) = directed_caps_mbps(&meta, &caps, &cfg, "sta", "ap").unwrap();
        assert_eq!(cap_ab, 50);
        assert_eq!(cap_ba, 180);

        // Argument order swap should swap the returned directed capacities accordingly.
        let (cap_ab, cap_ba) = directed_caps_mbps(&meta, &caps, &cfg, "ap", "sta").unwrap();
        assert_eq!(cap_ab, 180);
        assert_eq!(cap_ba, 50);
    }

    #[test]
    fn role_based_orientation_works() {
        let ap = mk_raw_device("ap", Some("ap"), None, None);
        let sta = mk_raw_device("sta", Some("station"), None, None);
        let meta = build_device_link_meta_map(&[ap, sta]);

        let cfg = Config::default();
        let caps = caps_map(&[("ap", 300, 80), ("sta", 250, 120)]);

        let (cap_ab, cap_ba) = directed_caps_mbps(&meta, &caps, &cfg, "ap", "sta").unwrap();
        assert_eq!(cap_ab, 250); // down=min(300,250)
        assert_eq!(cap_ba, 80); // up=min(80,120)
    }

    #[test]
    fn wireless_mode_based_orientation_works() {
        let ap = mk_raw_device("ap", None, Some("ap-ptmp"), None);
        let sta = mk_raw_device("sta", None, Some("sta-ptmp"), None);
        let meta = build_device_link_meta_map(&[ap, sta]);

        let cfg = Config::default();
        let caps = caps_map(&[("ap", 400, 90), ("sta", 350, 100)]);

        let (cap_ab, cap_ba) = directed_caps_mbps(&meta, &caps, &cfg, "ap", "sta").unwrap();
        assert_eq!(cap_ab, 350); // down=min(400,350)
        assert_eq!(cap_ba, 90); // up=min(90,100)
    }

    #[test]
    fn ambiguous_links_return_none() {
        let a = mk_raw_device("a", None, None, None);
        let b = mk_raw_device("b", None, None, None);
        let meta = build_device_link_meta_map(&[a, b]);

        let cfg = Config::default();
        let caps = caps_map(&[("a", 100, 100), ("b", 100, 100)]);

        assert!(directed_caps_mbps(&meta, &caps, &cfg, "a", "b").is_none());
    }

    #[test]
    fn mutual_ap_device_references_do_not_flip_orientation() {
        // UISP devices?withInterfaces=true reports apDevice on both ends of 1:1
        // PtP links; the labelled AP end must decide the orientation for both
        // argument orders.
        let ap = mk_raw_device("ap", Some("ap"), Some("ap-ptp"), Some("sta"));
        let sta = mk_raw_device("sta", Some("station"), Some("sta-ptp"), Some("ap"));
        let meta = build_device_link_meta_map(&[ap, sta]);
        let cfg = Config::default();
        let caps = caps_map(&[("ap", 193, 61), ("sta", 193, 61)]);
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "ap", "sta"),
            Some((193, 61))
        );
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "sta", "ap"),
            Some((61, 193))
        );
    }

    #[test]
    fn mutual_references_without_labels_are_order_independent() {
        // Both ends reference each other but neither has usable role/wireless
        // metadata. Decline in both argument orders instead of picking a side.
        let a = mk_raw_device("a", None, None, Some("b"));
        let b = mk_raw_device("b", None, None, Some("a"));
        let meta = build_device_link_meta_map(&[a, b]);
        let cfg = Config::default();
        let caps = caps_map(&[("a", 193, 61), ("b", 193, 61)]);
        assert!(classify_ap_station(&meta, "a", "b").is_none());
        assert!(classify_ap_station(&meta, "b", "a").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "a", "b").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "b", "a").is_none());
    }

    #[test]
    fn mutual_references_with_a_labelled_station_still_resolve() {
        // The AP end may have no role/wireless metadata; a positively-labelled
        // station end is enough to identify the other end as the AP.
        let ap = mk_raw_device("ap", None, None, Some("sta"));
        let sta = mk_raw_device("sta", Some("station"), Some("sta-ptp"), Some("ap"));
        let meta = build_device_link_meta_map(&[ap, sta]);
        let cfg = Config::default();
        let caps = caps_map(&[("ap", 193, 61), ("sta", 193, 61)]);
        assert_eq!(
            classify_ap_station(&meta, "ap", "sta"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station(&meta, "sta", "ap"),
            Some(LinkOrientation::BIsAp)
        );
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "ap", "sta"),
            Some((193, 61))
        );
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "sta", "ap"),
            Some((61, 193))
        );
    }

    #[test]
    fn both_ap_ends_are_ambiguous() {
        let ap1 = mk_raw_device("ap1", Some("ap"), Some("ap-ptp"), Some("ap2"));
        let ap2 = mk_raw_device("ap2", Some("ap"), Some("ap-ptp"), Some("ap1"));
        let meta = build_device_link_meta_map(&[ap1, ap2]);
        let cfg = Config::default();
        let caps = caps_map(&[("ap1", 193, 61), ("ap2", 193, 61)]);
        assert!(classify_ap_station(&meta, "ap1", "ap2").is_none());
        assert!(classify_ap_station(&meta, "ap2", "ap1").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "ap1", "ap2").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "ap2", "ap1").is_none());
    }

    #[test]
    fn one_way_ap_device_from_an_ap_uses_role_fallback() {
        let ap = mk_raw_device("ap", Some("ap"), Some("ap-ptp"), Some("sta"));
        let sta = mk_raw_device("sta", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[ap, sta]);
        assert_eq!(
            classify_ap_station(&meta, "ap", "sta"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station(&meta, "sta", "ap"),
            Some(LinkOrientation::BIsAp)
        );
    }

    #[test]
    fn mixed_role_and_wireless_uses_wireless_when_both_roles_are_station() {
        // Pre-existing fallback policy: when both explicit roles say station, the
        // wirelessMode pair decides. This is not a role-first precedence test.
        let a = mk_raw_device("a", Some("station"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        assert_eq!(
            classify_ap_station(&meta, "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
    }

    #[test]
    fn mutual_references_with_both_ends_station_are_ambiguous() {
        let a = mk_raw_device("a", Some("station"), Some("sta-ptp"), Some("b"));
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), Some("a"));
        let meta = build_device_link_meta_map(&[a, b]);
        let cfg = Config::default();
        let caps = caps_map(&[("a", 193, 61), ("b", 193, 61)]);
        assert!(classify_ap_station(&meta, "a", "b").is_none());
        assert!(classify_ap_station(&meta, "b", "a").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "a", "b").is_none());
        assert!(directed_caps_mbps(&meta, &caps, &cfg, "b", "a").is_none());
    }

    #[test]
    fn mutual_conflicting_labels_still_orient_by_the_ap_label() {
        // A positive AP label (wirelessMode here) wins even when the role says
        // station; the pair is still order-independent.
        let a = mk_raw_device("a", Some("station"), Some("ap-ptp"), Some("b"));
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), Some("a"));
        let meta = build_device_link_meta_map(&[a, b]);
        let cfg = Config::default();
        let caps = caps_map(&[("a", 193, 61), ("b", 193, 61)]);
        assert_eq!(
            classify_ap_station(&meta, "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station(&meta, "b", "a"),
            Some(LinkOrientation::BIsAp)
        );
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "a", "b"),
            Some((193, 61))
        );
        assert_eq!(
            directed_caps_mbps(&meta, &caps, &cfg, "b", "a"),
            Some((61, 193))
        );
    }

    #[test]
    fn role_pair_wins_over_conflicting_wireless_labels() {
        // No apDevice: the explicit role pair (ap,station) wins over the
        // conflicting wireless pair (sta,ap); this is the role-first policy.
        let a = mk_raw_device("a", Some("ap"), Some("sta-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("ap-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        assert_eq!(
            classify_ap_station(&meta, "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station(&meta, "b", "a"),
            Some(LinkOrientation::BIsAp)
        );
    }

    #[test]
    fn topology_north_site_wins_over_inverted_labels() {
        // The radio at the northern site is the AP even when both ends carry
        // misleading station/AP labels. This is the ISP-reported Wave case.
        let a = mk_raw_device("a", Some("station"), Some("sta-ptp"), Some("b"));
        let b = mk_raw_device("b", Some("ap"), Some("ap-ptp"), Some("a"));
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[("north", Some(40.0), None), ("south", Some(39.0), None)],
            &[("a", "north"), ("b", "south")],
        );
        let cfg = Config::default();
        let caps = caps_map(&[("a", 193, 61), ("b", 193, 61)]);

        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "b", "a"),
            Some(LinkOrientation::BIsAp)
        );
        assert_eq!(
            directed_caps_mbps_with_topology(&meta, Some(&topology), &caps, &cfg, "a", "b"),
            Some((193, 61))
        );
        assert_eq!(
            directed_caps_mbps_with_topology(&meta, Some(&topology), &caps, &cfg, "b", "a"),
            Some((61, 193))
        );
    }

    #[test]
    fn topology_parent_site_wins_over_north_site_and_labels() {
        // Site hierarchy is stronger than latitude: the upstream/parent site is
        // the AP even if the child site is further north.
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[
                ("parent", Some(39.0), None),
                ("child", Some(40.0), Some("parent")),
            ],
            &[("a", "child"), ("b", "parent")],
        );
        let cfg = Config::default();
        let caps = caps_map(&[("a", 193, 61), ("b", 193, 61)]);

        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::BIsAp)
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "b", "a"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            directed_caps_mbps_with_topology(&meta, Some(&topology), &caps, &cfg, "a", "b"),
            Some((61, 193))
        );
    }

    #[test]
    fn east_west_link_falls_back_to_labels() {
        // Same latitude (within the epsilon): there is no north/south winner,
        // so the existing label policy decides.
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[("east", Some(40.0), None), ("west", Some(40.0), None)],
            &[("a", "east"), ("b", "west")],
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "b", "a"),
            Some(LinkOrientation::BIsAp)
        );
    }

    #[test]
    fn latitude_noise_does_not_override_labels() {
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[
                ("a-site", Some(40.0), None),
                ("b-site", Some(40.000001), None),
            ],
            &[("a", "a-site"), ("b", "b-site")],
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
    }

    #[test]
    fn missing_site_coordinates_fall_back_to_labels() {
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[("a-site", None, None), ("b-site", None, None)],
            &[("a", "a-site"), ("b", "b-site")],
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
    }

    #[test]
    fn missing_device_site_assignment_falls_back_to_labels() {
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology =
            LinkTopology::from_test_sites(&[("a-site", Some(40.0), None)], &[("a", "a-site")]);
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
    }

    #[test]
    fn parent_cycle_does_not_hang_topology_resolution() {
        // Invalid UISP data: both sites point at each other. The visited set
        // must break the walk; the first parent edge still gives a stable answer.
        let a = mk_raw_device("a", Some("ap"), Some("ap-ptp"), None);
        let b = mk_raw_device("b", Some("station"), Some("sta-ptp"), None);
        let meta = build_device_link_meta_map(&[a, b]);
        let topology = LinkTopology::from_test_sites(
            &[
                ("a-site", Some(39.0), Some("b-site")),
                ("b-site", Some(40.0), Some("a-site")),
            ],
            &[("a", "a-site"), ("b", "b-site")],
        );
        assert_eq!(
            classify_ap_station_with_topology(&meta, Some(&topology), "a", "b"),
            Some(LinkOrientation::AIsAp)
        );
    }
}
