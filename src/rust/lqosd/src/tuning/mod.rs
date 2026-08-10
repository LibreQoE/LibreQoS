mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{Config, Tunables};
use lqos_queue_tracker::set_queue_refresh_interval;

#[derive(Debug, PartialEq, Eq)]
struct InterfaceTuningPlan {
    full: [String; 2],
    coalescing_only: Option<[String; 2]>,
}

fn interface_tuning_plan(config: &Config) -> InterfaceTuningPlan {
    let coalescing_only = config
        .bridge
        .as_ref()
        .filter(|bridge| bridge.compatibility_shim_enabled())
        .map(|bridge| [bridge.to_internet.clone(), bridge.to_network.clone()]);
    InterfaceTuningPlan {
        full: [config.internet_interface(), config.isp_interface()],
        coalescing_only,
    }
}

fn apply_non_interface_tuning(tuning: &Tunables) {
    offloads::bpf_sysctls();
    if tuning.set_cpu_governor_performance {
        offloads::set_cpu_governor_performance();
    }
    if tuning.stop_irq_balance {
        offloads::stop_irq_balance();
    }
    offloads::netdev_budget(tuning.netdev_budget_usecs, tuning.netdev_budget_packets);
}

fn apply_interface_tuning(config: &Config, tuning: &Tunables) {
    let plan = interface_tuning_plan(config);
    for interface in &plan.full {
        offloads::ethtool_tweaks(interface, tuning);
    }
    if let Some(physical_interfaces) = plan.coalescing_only {
        // Keep checksum, segmentation, and VLAN offloads enabled on the
        // physical path, but retain the configured interrupt-coalescing tune.
        for interface in &physical_interfaces {
            offloads::ethtool_coalescing_tweaks(interface, tuning);
        }
    }
}

pub fn tune_lqosd_from_config_file() -> Result<()> {
    let config = lqos_config::load_config()?;
    apply_non_interface_tuning(&config.tuning);
    apply_interface_tuning(config.as_ref(), &config.tuning);
    set_queue_refresh_interval(config.queue_check_period_ms);
    Ok(())
}

pub fn tune_lqosd_from_bus(request: &BusRequest) -> BusResponse {
    match request {
        BusRequest::UpdateLqosDTuning(interval, tuning) => {
            apply_non_interface_tuning(tuning);
            if let Ok(config) = lqos_config::load_config() {
                apply_interface_tuning(config.as_ref(), tuning);
            }
            set_queue_refresh_interval(*interval);
            lqos_bus::BusResponse::Ack
        }
        _ => BusResponse::Fail("That wasn't a tuning request".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::interface_tuning_plan;
    use lqos_config::{BridgeConfig, Config, SHIM_INTERNET_LQOS, SHIM_NETWORK_LQOS};

    fn bridge_config(compatibility_shim: bool) -> Config {
        Config {
            bridge: Some(BridgeConfig {
                use_xdp_bridge: true,
                to_internet: "bond0".to_string(),
                to_network: "enp2s0".to_string(),
                compatibility_shim,
                ..BridgeConfig::default()
            }),
            ..Config::default()
        }
    }

    #[test]
    fn compatibility_shim_keeps_full_offload_tuning_on_veths() {
        let direct = interface_tuning_plan(&bridge_config(false));
        assert_eq!(direct.full, ["bond0", "enp2s0"]);
        assert_eq!(direct.coalescing_only, None);

        let shim = interface_tuning_plan(&bridge_config(true));
        assert_eq!(shim.full, [SHIM_INTERNET_LQOS, SHIM_NETWORK_LQOS]);
        assert_eq!(
            shim.coalescing_only,
            Some(["bond0".to_string(), "enp2s0".to_string()])
        );
    }
}
