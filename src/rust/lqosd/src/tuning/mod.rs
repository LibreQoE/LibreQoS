mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{EtcLqos, LibreQoSConfig};
use crate::queue_tracker::QUEUE_MONITOR_INTERVAL;

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
    QUEUE_MONITOR_INTERVAL.store(interval, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

pub async fn tune_lqosd_from_bus(request: &BusRequest) -> BusResponse {
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
            QUEUE_MONITOR_INTERVAL.store(*interval, std::sync::atomic::Ordering::Relaxed);
            lqos_bus::BusResponse::Ack
        }
        _ => BusResponse::Fail("That wasn't a tuning request".to_string())
    }
}