mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{Config, Tunables};

fn apply_non_interface_tuning(tuning: &Tunables) {
    offloads::bpf_sysctls();
    // v3: `[power]` owns the CPU governor/EPP/turbo posture. The legacy
    // `[tuning] set_cpu_governor_performance` key is parsed for compatibility
    // but MUST NOT run after `libreqos-power` (it silently overrode the
    // configured profile and forced `performance` on .12).
    if tuning.set_cpu_governor_performance {
        tracing::warn!(
            setting = "tuning.set_cpu_governor_performance",
            "deprecated in v3: [power] owns the CPU governor; ignoring the legacy tuning key"
        );
    }
    if tuning.stop_irq_balance {
        offloads::stop_irq_balance();
    }
    offloads::netdev_budget(tuning.netdev_budget_usecs, tuning.netdev_budget_packets);
}

fn apply_interface_tuning(config: &Config, tuning: &Tunables) {
    offloads::ethtool_tweaks(&config.internet_interface(), tuning);
    offloads::ethtool_tweaks(&config.isp_interface(), tuning);
}

pub fn tune_lqosd_from_config_file() -> Result<()> {
    let config = lqos_config::load_config()?;
    apply_non_interface_tuning(&config.tuning);
    apply_interface_tuning(config.as_ref(), &config.tuning);
    Ok(())
}

pub fn tune_lqosd_from_bus(request: &BusRequest) -> BusResponse {
    match request {
        BusRequest::UpdateLqosDTuning(_interval, tuning) => {
            apply_non_interface_tuning(tuning);
            if let Ok(config) = lqos_config::load_config() {
                apply_interface_tuning(config.as_ref(), tuning);
            }
            lqos_bus::BusResponse::Ack
        }
        _ => BusResponse::Fail("That wasn't a tuning request".to_string()),
    }
}
