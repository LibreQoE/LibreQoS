use crate::errors::UispIntegrationError;
use crate::uisp_types::UispDevice;
use lqos_config::{CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadata, Config};
use std::path::Path;
use tracing::error;

const ETH_PORT_HEADROOM_FACTOR_SUB_GIG: f32 = 0.95;

/// Requested and applied circuit rates after negotiated-Ethernet evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct EthernetRateDecision {
    /// Minimum download after any cap was applied.
    pub download_min: f32,
    /// Minimum upload after any cap was applied.
    pub upload_min: f32,
    /// Maximum download after any cap was applied.
    pub download_max: f32,
    /// Maximum upload after any cap was applied.
    pub upload_max: f32,
    /// Optional UI advisory for the affected circuit.
    pub advisory: Option<CircuitEthernetMetadata>,
}

/// Applies a negotiated-Ethernet cap to a circuit and returns the adjusted rates plus advisory.
pub fn apply_ethernet_rate_cap<'a>(
    circuit_id: &str,
    circuit_name: &str,
    devices: impl IntoIterator<Item = &'a UispDevice>,
    download_min: f32,
    upload_min: f32,
    download_max: f32,
    upload_max: f32,
) -> EthernetRateDecision {
    let devices: Vec<&UispDevice> = devices.into_iter().collect();
    let related_device_ids = devices.iter().map(|device| device.id.clone()).collect();
    let candidate_devices: Vec<&UispDevice> = {
        let station_devices: Vec<&UispDevice> = devices
            .iter()
            .copied()
            .filter(|device| device.is_wireless_station_cpe())
            .collect();
        if !station_devices.is_empty() {
            station_devices
        } else {
            devices
                .iter()
                .copied()
                .filter(|device| !device.is_router_like())
                .collect()
        }
    };
    let mut limiting_device: Option<&UispDevice> = None;
    for device in &candidate_devices {
        let Some(speed_mbps) = device.negotiated_ethernet_mbps else {
            continue;
        };
        if limiting_device
            .as_ref()
            .is_none_or(|current| speed_mbps < current.negotiated_ethernet_mbps.unwrap_or(u64::MAX))
        {
            limiting_device = Some(device);
        }
    }

    let Some(limiting_device) = limiting_device else {
        return EthernetRateDecision {
            download_min,
            upload_min,
            download_max,
            upload_max,
            advisory: None,
        };
    };

    let usable_cap = ethernet_usable_cap_mbps(
        limiting_device
            .negotiated_ethernet_mbps
            .expect("limiting device must have ethernet speed"),
    );
    let applied_download_max = download_max.min(usable_cap);
    let applied_upload_max = upload_max.min(usable_cap);
    let applied_download_min = download_min.min(applied_download_max);
    let applied_upload_min = upload_min.min(applied_upload_max);
    let auto_capped = applied_download_max < download_max || applied_upload_max < upload_max;

    EthernetRateDecision {
        download_min: applied_download_min,
        upload_min: applied_upload_min,
        download_max: applied_download_max,
        upload_max: applied_upload_max,
        advisory: Some(CircuitEthernetMetadata {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_name.to_string(),
            device_ids: related_device_ids,
            source: "uisp".to_string(),
            negotiated_ethernet_mbps: limiting_device.negotiated_ethernet_mbps.unwrap_or_default(),
            requested_download_mbps: download_max,
            requested_upload_mbps: upload_max,
            applied_download_mbps: applied_download_max,
            applied_upload_mbps: applied_upload_max,
            auto_capped,
            limiting_device_id: Some(limiting_device.id.clone()),
            limiting_device_name: Some(limiting_device.name.clone()),
            limiting_interface_name: limiting_device.negotiated_ethernet_interface.clone(),
        }),
    }
}

/// Persists circuit Ethernet advisories alongside other generated runtime files.
pub fn write_ethernet_advisories(
    config: &Config,
    advisories: &[CircuitEthernetMetadata],
) -> Result<(), UispIntegrationError> {
    let path = Path::new(&config.lqos_directory).join(CIRCUIT_ETHERNET_METADATA_FILENAME);
    let payload = serde_json::to_vec_pretty(&lqos_config::CircuitEthernetMetadataFile {
        circuits: advisories
            .iter()
            .filter(|advisory| advisory.auto_capped)
            .cloned()
            .collect(),
    })
    .map_err(|e| {
        error!("Unable to serialize circuit Ethernet metadata: {e:?}");
        UispIntegrationError::CsvError
    })?;
    std::fs::write(path, payload).map_err(|e| {
        error!("Unable to write circuit Ethernet metadata: {e:?}");
        UispIntegrationError::CsvError
    })?;
    Ok(())
}

fn ethernet_usable_cap_mbps(negotiated_mbps: u64) -> f32 {
    match negotiated_mbps {
        0..=999 => negotiated_mbps as f32 * ETH_PORT_HEADROOM_FACTOR_SUB_GIG,
        1000..=2499 => negotiated_mbps.saturating_sub(5) as f32,
        2500..=9999 => negotiated_mbps.saturating_sub(10) as f32,
        _ => negotiated_mbps.saturating_sub(50) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_ethernet_rate_cap, write_ethernet_advisories};
    use crate::uisp_types::UispDevice;
    use lqos_config::{CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadataFile, Config};
    use std::collections::HashSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_device(
        id: &str,
        name: &str,
        speed: Option<u64>,
        iface: Option<&str>,
        role: Option<&str>,
        wireless_mode: Option<&str>,
    ) -> UispDevice {
        UispDevice {
            id: id.to_string(),
            name: name.to_string(),
            mac: String::new(),
            role: role.map(str::to_string),
            wireless_mode: wireless_mode.map(str::to_string),
            site_id: "site-1".to_string(),
            download: 1000,
            upload: 1000,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            negotiated_ethernet_mbps: speed,
            negotiated_ethernet_interface: iface.map(str::to_string),
        }
    }

    #[test]
    fn ethernet_cap_uses_lowest_detected_port_speed() {
        let fast = sample_device(
            "dev-fast",
            "Fast",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );
        let slow = sample_device(
            "dev-slow",
            "Slow",
            Some(100),
            Some("eth1"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision = apply_ethernet_rate_cap(
            "circuit-1",
            "Circuit 1",
            [&fast, &slow],
            25.0,
            25.0,
            300.0,
            300.0,
        );

        assert_eq!(decision.download_max, 95.0);
        assert_eq!(decision.upload_max, 95.0);
        assert_eq!(decision.download_min, 25.0);
        assert_eq!(decision.upload_min, 25.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.negotiated_ethernet_mbps, 100);
        assert_eq!(advisory.limiting_device_id.as_deref(), Some("dev-slow"));
    }

    #[test]
    fn ethernet_cap_preserves_lower_upload_plan() {
        let slow = sample_device(
            "dev-slow",
            "Slow",
            Some(100),
            Some("eth1"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision =
            apply_ethernet_rate_cap("circuit-1", "Circuit 1", [&slow], 10.0, 10.0, 300.0, 50.0);

        assert_eq!(decision.download_max, 95.0);
        assert_eq!(decision.upload_max, 50.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 95.0);
        assert_eq!(advisory.applied_upload_mbps, 50.0);
    }

    #[test]
    fn gigabit_ports_use_small_fixed_margin() {
        let gig = sample_device(
            "dev-gig",
            "Gig",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision =
            apply_ethernet_rate_cap("circuit-1", "Circuit 1", [&gig], 100.0, 100.0, 995.0, 995.0);

        assert_eq!(decision.download_max, 995.0);
        assert_eq!(decision.upload_max, 995.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(!advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 995.0);
        assert_eq!(advisory.applied_upload_mbps, 995.0);
    }

    #[test]
    fn gigabit_ports_cap_above_line_rate_to_995() {
        let gig = sample_device(
            "dev-gig",
            "Gig",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision = apply_ethernet_rate_cap(
            "circuit-1",
            "Circuit 1",
            [&gig],
            100.0,
            100.0,
            5000.0,
            5000.0,
        );

        assert_eq!(decision.download_max, 995.0);
        assert_eq!(decision.upload_max, 995.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 995.0);
        assert_eq!(advisory.applied_upload_mbps, 995.0);
    }

    #[test]
    fn station_devices_win_over_router_devices() {
        let station = sample_device(
            "dev-station",
            "Station",
            Some(1000),
            Some("data"),
            Some("station"),
            Some("sta-ptmp"),
        );
        let router = sample_device(
            "dev-router",
            "Router",
            Some(10),
            Some("eth3"),
            Some("homeWiFi"),
            Some("ap"),
        );

        let decision = apply_ethernet_rate_cap(
            "circuit-1",
            "Circuit 1",
            [&station, &router],
            10.0,
            10.0,
            115.0,
            22.0,
        );

        assert_eq!(decision.download_max, 115.0);
        assert_eq!(decision.upload_max, 22.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(!advisory.auto_capped);
        assert_eq!(advisory.negotiated_ethernet_mbps, 1000);
        assert_eq!(advisory.limiting_device_id.as_deref(), Some("dev-station"));
    }

    #[test]
    fn router_only_devices_do_not_drive_ethernet_cap() {
        let router = sample_device(
            "dev-router",
            "Router",
            Some(100),
            Some("eth0"),
            Some("router"),
            None,
        );

        let decision =
            apply_ethernet_rate_cap("circuit-1", "Circuit 1", [&router], 10.0, 10.0, 115.0, 22.0);

        assert_eq!(decision.download_max, 115.0);
        assert_eq!(decision.upload_max, 22.0);
        assert!(decision.advisory.is_none());
    }

    #[test]
    fn writer_persists_only_auto_capped_advisories() {
        let auto_capped = sample_device(
            "dev-100m",
            "Hundred",
            Some(100),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );
        let observed_only = sample_device(
            "dev-1g",
            "Gig",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let capped = apply_ethernet_rate_cap(
            "circuit-capped",
            "Capped Circuit",
            [&auto_capped],
            10.0,
            10.0,
            115.0,
            22.0,
        )
        .advisory
        .expect("auto-capped advisory should exist");
        let observed = apply_ethernet_rate_cap(
            "circuit-observed",
            "Observed Circuit",
            [&observed_only],
            100.0,
            100.0,
            995.0,
            995.0,
        )
        .advisory
        .expect("observed advisory should exist");

        let mut config = Config::default();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("lqos-ethernet-writer-{unique}"));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        config.lqos_directory = temp_dir.to_string_lossy().to_string();

        write_ethernet_advisories(&config, &[capped.clone(), observed])
            .expect("writer should succeed");

        let payload = fs::read(temp_dir.join(CIRCUIT_ETHERNET_METADATA_FILENAME))
            .expect("metadata file should exist");
        let parsed: CircuitEthernetMetadataFile =
            serde_json::from_slice(&payload).expect("metadata file should parse");
        assert_eq!(parsed.circuits, vec![capped]);

        let _ = fs::remove_dir_all(temp_dir);
    }
}
