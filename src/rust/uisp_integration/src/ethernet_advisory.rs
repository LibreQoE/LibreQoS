use crate::errors::UispIntegrationError;
use crate::uisp_types::UispDevice;
use lqos_config::{
    CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadata, Config, EthernetPortLimitPolicy,
    EthernetPortObservation, EthernetRateDecision, RequestedCircuitRates,
    apply_ethernet_rate_cap as apply_shared_ethernet_rate_cap,
};
use tracing::error;

/// Applies a negotiated-Ethernet cap to a circuit and returns the adjusted rates plus advisory.
pub fn apply_ethernet_rate_cap<'a>(
    policy: EthernetPortLimitPolicy,
    circuit_id: &str,
    circuit_name: &str,
    devices: impl IntoIterator<Item = &'a UispDevice>,
    requested_rates: RequestedCircuitRates,
) -> EthernetRateDecision {
    let observations = build_ethernet_port_observations(devices);
    apply_shared_ethernet_rate_cap(
        policy,
        circuit_id,
        circuit_name,
        observations.iter(),
        requested_rates,
    )
}

fn build_ethernet_port_observations<'a>(
    devices: impl IntoIterator<Item = &'a UispDevice>,
) -> Vec<EthernetPortObservation> {
    let devices: Vec<&UispDevice> = devices.into_iter().collect();
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

    candidate_devices
        .into_iter()
        .filter_map(|device| {
            Some(EthernetPortObservation {
                source: "uisp".to_string(),
                device_id: Some(device.id.clone()),
                device_name: Some(device.name.clone()),
                interface_name: device.negotiated_ethernet_interface.clone(),
                negotiated_ethernet_mbps: device.negotiated_ethernet_mbps?,
            })
        })
        .collect()
}

/// Persists circuit Ethernet advisories alongside other generated runtime files.
pub fn write_ethernet_advisories(
    config: &Config,
    advisories: &[CircuitEthernetMetadata],
) -> Result<(), UispIntegrationError> {
    let path = config.topology_state_file_path(CIRCUIT_ETHERNET_METADATA_FILENAME);
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Unable to create circuit Ethernet metadata directory: {e:?}");
            UispIntegrationError::CsvError
        })?;
    }
    std::fs::write(path, payload).map_err(|e| {
        error!("Unable to write circuit Ethernet metadata: {e:?}");
        UispIntegrationError::CsvError
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_ethernet_rate_cap, write_ethernet_advisories};
    use crate::uisp_types::UispDevice;
    use lqos_config::{
        CIRCUIT_ETHERNET_METADATA_FILENAME, CircuitEthernetMetadataFile, Config,
        EthernetPortLimitPolicy, RequestedCircuitRates,
    };
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
            raw_download: 1000,
            raw_upload: 1000,
            download: 1000,
            upload: 1000,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            probe_ipv4: HashSet::new(),
            probe_ipv6: HashSet::new(),
            negotiated_ethernet_mbps: speed,
            negotiated_ethernet_interface: iface.map(str::to_string),
            transport_cap_mbps: None,
            transport_cap_reason: None,
            attachment_rate_source: crate::uisp_types::UispAttachmentRateSource::Static,
        }
    }

    #[test]
    fn ethernet_cap_uses_lowest_detected_port_speed() {
        let policy = EthernetPortLimitPolicy::default();
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
            policy,
            "circuit-1",
            "Circuit 1",
            [&fast, &slow],
            RequestedCircuitRates {
                download_min: 25.0,
                upload_min: 25.0,
                download_max: 300.0,
                upload_max: 300.0,
            },
        );

        assert_eq!(decision.download_max, 94.0);
        assert_eq!(decision.upload_max, 94.0);
        assert_eq!(decision.download_min, 25.0);
        assert_eq!(decision.upload_min, 25.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.negotiated_ethernet_mbps, 100);
        assert_eq!(advisory.limiting_device_id.as_deref(), Some("dev-slow"));
    }

    #[test]
    fn ethernet_cap_preserves_lower_upload_plan() {
        let policy = EthernetPortLimitPolicy::default();
        let slow = sample_device(
            "dev-slow",
            "Slow",
            Some(100),
            Some("eth1"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision = apply_ethernet_rate_cap(
            policy,
            "circuit-1",
            "Circuit 1",
            [&slow],
            RequestedCircuitRates {
                download_min: 10.0,
                upload_min: 10.0,
                download_max: 300.0,
                upload_max: 50.0,
            },
        );

        assert_eq!(decision.download_max, 94.0);
        assert_eq!(decision.upload_max, 50.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 94.0);
        assert_eq!(advisory.applied_upload_mbps, 50.0);
    }

    #[test]
    fn gigabit_ports_use_shared_94_percent_default() {
        let policy = EthernetPortLimitPolicy::default();
        let gig = sample_device(
            "dev-gig",
            "Gig",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision = apply_ethernet_rate_cap(
            policy,
            "circuit-1",
            "Circuit 1",
            [&gig],
            RequestedCircuitRates {
                download_min: 100.0,
                upload_min: 100.0,
                download_max: 995.0,
                upload_max: 995.0,
            },
        );

        assert_eq!(decision.download_max, 940.0);
        assert_eq!(decision.upload_max, 940.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 940.0);
        assert_eq!(advisory.applied_upload_mbps, 940.0);
    }

    #[test]
    fn operator_override_can_cap_gigabit_to_custom_multiplier() {
        let policy = EthernetPortLimitPolicy {
            enabled: true,
            multiplier: 0.9,
        };
        let gig = sample_device(
            "dev-gig",
            "Gig",
            Some(1000),
            Some("eth0"),
            Some("station"),
            Some("sta-ptmp"),
        );

        let decision = apply_ethernet_rate_cap(
            policy,
            "circuit-1",
            "Circuit 1",
            [&gig],
            RequestedCircuitRates {
                download_min: 100.0,
                upload_min: 100.0,
                download_max: 5000.0,
                upload_max: 5000.0,
            },
        );

        assert_eq!(decision.download_max, 900.0);
        assert_eq!(decision.upload_max, 900.0);
        let advisory = decision.advisory.expect("advisory should exist");
        assert!(advisory.auto_capped);
        assert_eq!(advisory.applied_download_mbps, 900.0);
        assert_eq!(advisory.applied_upload_mbps, 900.0);
    }

    #[test]
    fn station_devices_win_over_router_devices() {
        let policy = EthernetPortLimitPolicy::default();
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
            policy,
            "circuit-1",
            "Circuit 1",
            [&station, &router],
            RequestedCircuitRates {
                download_min: 10.0,
                upload_min: 10.0,
                download_max: 115.0,
                upload_max: 22.0,
            },
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
        let policy = EthernetPortLimitPolicy::default();
        let router = sample_device(
            "dev-router",
            "Router",
            Some(100),
            Some("eth0"),
            Some("router"),
            None,
        );

        let decision = apply_ethernet_rate_cap(
            policy,
            "circuit-1",
            "Circuit 1",
            [&router],
            RequestedCircuitRates {
                download_min: 10.0,
                upload_min: 10.0,
                download_max: 115.0,
                upload_max: 22.0,
            },
        );

        assert_eq!(decision.download_max, 115.0);
        assert_eq!(decision.upload_max, 22.0);
        assert!(decision.advisory.is_none());
    }

    #[test]
    fn writer_persists_only_auto_capped_advisories() {
        let policy = EthernetPortLimitPolicy::default();
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
            policy,
            "circuit-capped",
            "Capped Circuit",
            [&auto_capped],
            RequestedCircuitRates {
                download_min: 10.0,
                upload_min: 10.0,
                download_max: 115.0,
                upload_max: 22.0,
            },
        )
        .advisory
        .expect("auto-capped advisory should exist");
        let observed = apply_ethernet_rate_cap(
            policy,
            "circuit-observed",
            "Observed Circuit",
            [&observed_only],
            RequestedCircuitRates {
                download_min: 100.0,
                upload_min: 100.0,
                download_max: 900.0,
                upload_max: 900.0,
            },
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
