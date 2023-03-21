mod lshw;
mod version;
use std::{time::Duration, net::TcpStream, io::Write};
use lqos_bus::anonymous::{AnonymousUsageV1, build_stats};
use lqos_config::{EtcLqos, LibreQoSConfig};
use lqos_sys::libbpf_num_possible_cpus;
use sysinfo::{System, SystemExt, CpuExt};
use crate::{shaped_devices_tracker::{SHAPED_DEVICES, NETWORK_JSON}, stats::{HIGH_WATERMARK_DOWN, HIGH_WATERMARK_UP}};

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
    let mut server = String::new();
    sys.refresh_all();
    data.total_memory = sys.total_memory();
    data.available_memory = sys.available_memory();
    if let Some(kernel) = sys.kernel_version() {
        data.kernel_version = kernel;
    }
    data.usable_cores = unsafe { libbpf_num_possible_cpus() } as u32;
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
        data.on_a_stick = cfg.on_a_stick_mode;
    }

    if let Ok(cfg) = EtcLqos::load() {
        if let Some(node_id) = cfg.node_id {
            data.node_id = node_id;
            if let Some(bridge) = cfg.bridge {
                data.using_xdp_bridge = bridge.use_xdp_bridge;
            }
        }
        if let Some(anon) = cfg.usage_stats {
            server = anon.anonymous_server;
        }
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