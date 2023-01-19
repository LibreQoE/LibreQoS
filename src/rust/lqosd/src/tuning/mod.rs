mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{EtcLqos, LibreQoSConfig};
use lqos_queue_tracker::set_queue_refresh_interval;

pub fn tune_lqosd_from_config_file(config: &LibreQoSConfig) -> Result<()> {
    let etc_lqos = EtcLqos::load()?;

    // Disable offloading
    if let Some(tuning) = &etc_lqos.tuning {
        offloads::bpf_sysctls();
        if tuning.stop_irq_balance {
            offloads::stop_irq_balance();
        }
        offloads::netdev_budget(tuning.netdev_budget_usecs, tuning.netdev_budget_packets);
        offloads::ethtool_tweaks(&config.internet_interface, tuning);
        offloads::ethtool_tweaks(&config.isp_interface, tuning);
    }
    let interval = etc_lqos.queue_check_period_ms;
    set_queue_refresh_interval(interval);
    Ok(())
}

pub fn tune_lqosd_from_bus(request: &BusRequest) -> BusResponse {
    match request {
        BusRequest::UpdateLqosDTuning(interval, tuning) => {
            // Real-time tuning changes. Probably dangerous.
            if let Ok(config) = LibreQoSConfig::load() {
                if tuning.stop_irq_balance {
                    offloads::stop_irq_balance();
                }
                offloads::netdev_budget(tuning.netdev_budget_usecs, tuning.netdev_budget_packets);
                offloads::ethtool_tweaks(&config.internet_interface, tuning);
                offloads::ethtool_tweaks(&config.isp_interface, tuning);
            }
            set_queue_refresh_interval(*interval);
            lqos_bus::BusResponse::Ack
        }
        _ => BusResponse::Fail("That wasn't a tuning request".to_string()),
    }
}
