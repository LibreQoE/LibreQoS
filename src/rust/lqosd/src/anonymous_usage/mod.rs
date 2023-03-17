mod lshw;
use std::time::Duration;
use lqos_bus::anonymous::AnonymousUsageV1;
use lqos_config::{EtcLqos, LibreQoSConfig};
use lqos_sys::num_possible_cpus;
use sysinfo::{System, SystemExt, CpuExt};

use crate::shaped_devices_tracker::{SHAPED_DEVICES, NETWORK_JSON};

const SLOW_START_SECS: u64 = 1;
const INTERVAL_SECS: u64 = 60 * 60 * 24;

pub async fn start_anonymous_usage() {
    if let Ok(cfg) = EtcLqos::load() {
        if let Some(usage) = cfg.usage_stats {
            if usage.send_anonymous {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_secs(SLOW_START_SECS));
                    loop {
                        let _ = anonymous_usage_dump();
                        std::thread::sleep(Duration::from_secs(INTERVAL_SECS));
                    }
                });
            }
        }
    }
}

fn anonymous_usage_dump() -> anyhow::Result<()> {
    let mut data = AnonymousUsageV1::default();
    let mut sys = System::new_all();
    sys.refresh_all();
    data.total_memory = sys.total_memory();
    data.available_memory = sys.available_memory();
    if let Some(kernel) = sys.kernel_version() {
        data.kernel_version = kernel;
    }
    if let Ok(cores) = num_possible_cpus() {
        data.usable_cores = cores;
    }
    let cpu = sys.cpus().first();
    if let Some(cpu) = cpu {
        data.cpu_brand = cpu.brand().to_string();
        data.cpu_vendor = cpu.vendor_id().to_string();
        data.cpu_frequency = cpu.frequency();
    }
    for nic in lshw::get_nic_info()? {
        data.nics.push(nic.into());
    }

    if let Ok(cfg) = LibreQoSConfig::load() {
        data.sqm = cfg.sqm;
        data.monitor_mode = cfg.monitor_mode;
        data.total_capacity = (
            cfg.total_download_mbps,
            cfg.total_upload_mbps,
        );
        data.generated_pdn_capacity = (
            cfg.generated_download_mbps,
            cfg.generated_upload_mbps,
        );
    }

    data.shaped_device_count = SHAPED_DEVICES.read().unwrap().devices.len();
    data.net_json_len = NETWORK_JSON.read().unwrap().nodes.len();

    println!("{data:#?}");
    Ok(())
}