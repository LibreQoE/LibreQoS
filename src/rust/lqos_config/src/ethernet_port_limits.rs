use crate::CircuitEthernetMetadata;
use crate::etc::IntegrationConfig;
use serde::{Deserialize, Serialize};

/// Default multiplier used to translate negotiated Ethernet line rate into a
/// conservative shaping ceiling that leaves room for framing and transport overhead.
pub const DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER: f32 = 0.94;

/// One integration-provided Ethernet observation for a circuit candidate device/interface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EthernetPortObservation {
    /// Integration/source that produced the observation.
    pub source: String,
    /// Optional limiting device identifier.
    pub device_id: Option<String>,
    /// Optional limiting device display name.
    pub device_name: Option<String>,
    /// Optional interface name that reported the negotiated speed.
    pub interface_name: Option<String>,
    /// Observed negotiated Ethernet speed in Mbps.
    pub negotiated_ethernet_mbps: u64,
}

/// Shared circuit-level Ethernet cap policy derived from configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EthernetPortLimitPolicy {
    /// Whether integration-driven Ethernet capping is enabled.
    pub enabled: bool,
    /// Multiplier used to translate negotiated line rate into usable shaping rate.
    pub multiplier: f32,
}

impl Default for EthernetPortLimitPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            multiplier: DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER,
        }
    }
}

impl From<&IntegrationConfig> for EthernetPortLimitPolicy {
    fn from(config: &IntegrationConfig) -> Self {
        Self {
            enabled: config.ethernet_port_limits_enabled,
            multiplier: config
                .ethernet_port_limit_multiplier
                .unwrap_or(DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER),
        }
    }
}

/// Requested circuit shaping rates before any Ethernet-port cap is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RequestedCircuitRates {
    /// Requested minimum download rate in Mbps.
    pub download_min: f32,
    /// Requested minimum upload rate in Mbps.
    pub upload_min: f32,
    /// Requested maximum download rate in Mbps.
    pub download_max: f32,
    /// Requested maximum upload rate in Mbps.
    pub upload_max: f32,
}

/// Requested and applied circuit rates after Ethernet-port evaluation.
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
    /// Optional UI/runtime advisory for the affected circuit.
    pub advisory: Option<CircuitEthernetMetadata>,
}

/// Applies a shared Ethernet cap policy to one circuit using normalized device/interface observations.
pub fn apply_ethernet_rate_cap<'a>(
    policy: EthernetPortLimitPolicy,
    circuit_id: &str,
    circuit_name: &str,
    observations: impl IntoIterator<Item = &'a EthernetPortObservation>,
    requested_rates: RequestedCircuitRates,
) -> EthernetRateDecision {
    let RequestedCircuitRates {
        download_min,
        upload_min,
        download_max,
        upload_max,
    } = requested_rates;

    if !policy.enabled {
        return EthernetRateDecision {
            download_min,
            upload_min,
            download_max,
            upload_max,
            advisory: None,
        };
    }

    let observations: Vec<&EthernetPortObservation> = observations
        .into_iter()
        .filter(|observation| observation.negotiated_ethernet_mbps > 0)
        .collect();
    let Some(limiting_observation) = observations
        .iter()
        .copied()
        .min_by_key(|observation| observation.negotiated_ethernet_mbps)
    else {
        return EthernetRateDecision {
            download_min,
            upload_min,
            download_max,
            upload_max,
            advisory: None,
        };
    };

    let usable_cap = limiting_observation.negotiated_ethernet_mbps as f32 * policy.multiplier;
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
            device_ids: unique_device_ids(&observations),
            source: limiting_observation.source.clone(),
            negotiated_ethernet_mbps: limiting_observation.negotiated_ethernet_mbps,
            requested_download_mbps: download_max,
            requested_upload_mbps: upload_max,
            applied_download_mbps: applied_download_max,
            applied_upload_mbps: applied_upload_max,
            auto_capped,
            limiting_device_id: limiting_observation.device_id.clone(),
            limiting_device_name: limiting_observation.device_name.clone(),
            limiting_interface_name: limiting_observation.interface_name.clone(),
        }),
    }
}

fn unique_device_ids(observations: &[&EthernetPortObservation]) -> Vec<String> {
    let mut ids = Vec::new();
    for observation in observations {
        let Some(device_id) = &observation.device_id else {
            continue;
        };
        if !ids.contains(device_id) {
            ids.push(device_id.clone());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER, EthernetPortLimitPolicy, EthernetPortObservation,
        RequestedCircuitRates, apply_ethernet_rate_cap,
    };

    fn observation(device_id: &str, speed: u64) -> EthernetPortObservation {
        EthernetPortObservation {
            source: "test".to_string(),
            device_id: Some(device_id.to_string()),
            device_name: Some(device_id.to_string()),
            interface_name: Some("eth0".to_string()),
            negotiated_ethernet_mbps: speed,
        }
    }

    #[test]
    fn default_policy_caps_hundred_meg_to_94_percent() {
        let observation = observation("dev-100m", 100);
        let decision = apply_ethernet_rate_cap(
            EthernetPortLimitPolicy::default(),
            "circuit-1",
            "Circuit 1",
            [&observation],
            RequestedCircuitRates {
                download_min: 10.0,
                upload_min: 10.0,
                download_max: 300.0,
                upload_max: 300.0,
            },
        );

        assert_eq!(decision.download_max, 94.0);
        assert_eq!(decision.upload_max, 94.0);
        assert_eq!(decision.download_min, 10.0);
        assert_eq!(decision.upload_min, 10.0);
        assert!(
            decision
                .advisory
                .as_ref()
                .is_some_and(|advisory| advisory.auto_capped)
        );
    }

    #[test]
    fn override_policy_applies_to_gigabit_ports() {
        let observation = observation("dev-gig", 1000);
        let decision = apply_ethernet_rate_cap(
            EthernetPortLimitPolicy {
                enabled: true,
                multiplier: 0.9,
            },
            "circuit-1",
            "Circuit 1",
            [&observation],
            RequestedCircuitRates {
                download_min: 100.0,
                upload_min: 100.0,
                download_max: 2000.0,
                upload_max: 2000.0,
            },
        );

        assert_eq!(decision.download_max, 900.0);
        assert_eq!(decision.upload_max, 900.0);
        assert!(
            decision
                .advisory
                .as_ref()
                .is_some_and(|advisory| advisory.auto_capped)
        );
    }

    #[test]
    fn lowest_observation_wins_for_circuit() {
        let gig = observation("dev-gig", 1000);
        let fast_ethernet = observation("dev-fast-ethernet", 100);
        let decision = apply_ethernet_rate_cap(
            EthernetPortLimitPolicy::default(),
            "circuit-1",
            "Circuit 1",
            [&gig, &fast_ethernet],
            RequestedCircuitRates {
                download_min: 50.0,
                upload_min: 50.0,
                download_max: 500.0,
                upload_max: 500.0,
            },
        );

        assert_eq!(decision.download_max, 94.0);
        assert_eq!(decision.upload_max, 94.0);
        assert_eq!(
            decision
                .advisory
                .as_ref()
                .and_then(|advisory| advisory.limiting_device_id.as_deref()),
            Some("dev-fast-ethernet")
        );
    }

    #[test]
    fn default_multiplier_constant_is_94_percent() {
        assert_eq!(DEFAULT_ETHERNET_PORT_LIMIT_MULTIPLIER, 0.94);
    }
}
