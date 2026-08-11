mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{Config, Tunables};
use lqos_queue_tracker::set_queue_refresh_interval;

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
    offloads::ethtool_tweaks(&config.internet_interface(), tuning);
    offloads::ethtool_tweaks(&config.isp_interface(), tuning);
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
