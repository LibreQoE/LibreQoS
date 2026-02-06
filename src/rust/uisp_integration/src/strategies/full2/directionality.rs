use crate::uisp_types::UispDevice;
use lqos_config::Config;
use std::collections::HashMap;
use uisp::Device;

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

pub(crate) fn build_device_link_meta_map(devices_raw: &[Device]) -> HashMap<String, DeviceLinkMeta> {
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
    let meta_a = meta_by_id.get(id_a)?;
    let meta_b = meta_by_id.get(id_b)?;

    // Highest confidence: the station references its AP by device ID.
    if meta_a.ap_device_id.as_deref() == Some(id_b) {
        return Some(LinkOrientation::BIsAp);
    }
    if meta_b.ap_device_id.as_deref() == Some(id_a) {
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
    let orientation = classify_ap_station(meta_by_id, id_a, id_b)?;

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
}

