mod offloads;
use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_queue_tracker::set_queue_refresh_interval;

pub fn tune_lqosd_from_config_file() -> Result<()> {
  let config = lqos_config::load_config()?;

  // Disable offloading
  offloads::bpf_sysctls();
  if config.tuning.stop_irq_balance {
    offloads::stop_irq_balance();
  }
  offloads::netdev_budget(
    config.tuning.netdev_budget_usecs,
    config.tuning.netdev_budget_packets,
  );
  offloads::ethtool_tweaks(&config.internet_interface(), &config.tuning);
  offloads::ethtool_tweaks(&config.isp_interface(), &config.tuning);
  let interval = config.queue_check_period_ms;
  set_queue_refresh_interval(interval);
  Ok(())
}

pub fn tune_lqosd_from_bus(request: &BusRequest) -> BusResponse {
  match request {
    BusRequest::UpdateLqosDTuning(interval, tuning) => {
      // Real-time tuning changes. Probably dangerous.
      if let Ok(config) = lqos_config::load_config() {
        if tuning.stop_irq_balance {
          offloads::stop_irq_balance();
        }
        offloads::netdev_budget(
          tuning.netdev_budget_usecs,
          tuning.netdev_budget_packets,
        );
        offloads::ethtool_tweaks(&config.internet_interface(), &config.tuning);
        offloads::ethtool_tweaks(&config.isp_interface(), &config.tuning);
      }
      set_queue_refresh_interval(*interval);
      lqos_bus::BusResponse::Ack
    }
    _ => BusResponse::Fail("That wasn't a tuning request".to_string()),
  }
}
