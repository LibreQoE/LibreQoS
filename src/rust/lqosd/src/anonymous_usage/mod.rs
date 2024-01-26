mod lshw;
mod version;
use std::{time::Duration, net::TcpStream, io::Write};
use lqos_bus::anonymous::{AnonymousUsageV1, build_stats};
use lqos_sys::num_possible_cpus;
use sysinfo::System;
use crate::{shaped_devices_tracker::{SHAPED_DEVICES, NETWORK_JSON}, stats::{HIGH_WATERMARK_DOWN, HIGH_WATERMARK_UP}};

const SLOW_START_SECS: u64 = 1;
const INTERVAL_SECS: u64 = 60 * 60 * 24;

pub async fn start_anonymous_usage() {
    if let Ok(cfg) = lqos_config::load_config() {
        if cfg.usage_stats.send_anonymous {
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

fn anonymous_usage_dump() -> anyhow::Result<()> {
    let mut data = AnonymousUsageV1::default();
    let mut sys = System::new_all();
    let mut server = String::new();
    sys.refresh_all();
    data.total_memory = sys.total_memory();
    data.available_memory = sys.available_memory();
    if let Some(kernel) = sysinfo::System::kernel_version() {
        data.kernel_version = kernel;
    }
    data.usable_cores = num_possible_cpus().unwrap_or(0);
    let cpu = sys.cpus().first();
    if let Some(cpu) = cpu {
        data.cpu_brand = cpu.brand().to_string();
        data.cpu_vendor = cpu.vendor_id().to_string();
        data.cpu_frequency = cpu.frequency();
    }
    if let Ok(nics) = lshw::get_nic_info() {
        for nic in nics {
            data.nics.push(nic.into());
        }
    }
    if let Ok(pv) = version::get_proc_version() {
        data.distro = pv.trim().to_string();
    }

    if let Ok(cfg) = lqos_config::load_config() {
        data.sqm = cfg.queues.default_sqm.clone();
        data.monitor_mode = cfg.queues.monitor_only;
        data.total_capacity = (
            cfg.queues.downlink_bandwidth_mbps,
            cfg.queues.uplink_bandwidth_mbps,
        );
        data.generated_pdn_capacity = (
            cfg.queues.generated_pn_download_mbps,
            cfg.queues.generated_pn_upload_mbps,
        );
        data.on_a_stick = cfg.on_a_stick_mode();

        data.node_id = cfg.node_id.clone();
        if let Some(bridge) = cfg.bridge {
            data.using_xdp_bridge = bridge.use_xdp_bridge;
        }
        server = cfg.usage_stats.anonymous_server;
    }

    data.git_hash = env!("GIT_HASH").to_string();
    data.shaped_device_count = SHAPED_DEVICES.read().unwrap().devices.len();
    data.net_json_len = NETWORK_JSON.read().unwrap().nodes.len();

    data.high_watermark_bps = (
        HIGH_WATERMARK_DOWN.load(std::sync::atomic::Ordering::Relaxed),
        HIGH_WATERMARK_UP.load(std::sync::atomic::Ordering::Relaxed),
    );


    send_stats(data, &server);
    Ok(())
}

fn send_stats(data: AnonymousUsageV1, server: &str) {
    let buffer = build_stats(&data);
    if let Err(e) = buffer {
        log::warn!("Unable to serialize stats buffer");
        log::warn!("{e:?}");
        return;
    }
    let buffer = buffer.unwrap();

    let stream = TcpStream::connect(server);
    if let Err(e) = stream {
        log::warn!("Unable to connect to {server}");
        log::warn!("{e:?}");
        return;
    }
    let mut stream = stream.unwrap();
    let result = stream.write(&buffer);
    if let Err(e) = result {
        log::warn!("Unable to send bytes to {server}");
        log::warn!("{e:?}");
    }
    log::info!("Anonymous usage stats submitted");
}