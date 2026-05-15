use crate::{EXPIRE_WATCHES_SECS, HeimdalConfig, HeimdallMode};
use dashmap::DashMap;
use lqos_sys::bpf_map::BpfMap;
use lqos_utils::{XdpIpAddress, unix_time::time_since_boot};
use once_cell::sync::Lazy;
use std::time::Duration;
use tracing::{debug, info, warn};

const HEIMDALL_CFG_PATH: &str = "/sys/fs/bpf/heimdall_config";
const HEIMDALL_WATCH_PATH: &str = "/sys/fs/bpf/heimdall_watching";

/// Change the eBPF Heimdall System mode.
pub fn set_heimdall_mode(mode: HeimdallMode) -> anyhow::Result<()> {
    let mut map = BpfMap::<u32, HeimdalConfig>::from_path(HEIMDALL_CFG_PATH)?;
    map.insert_or_update(&mut 0, &mut HeimdalConfig { mode: mode as u32 })?;
    Ok(())
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct HeimdallWatching {
    expiration: u128,
    ip_address: XdpIpAddress,
}

impl HeimdallWatching {
    pub fn new(mut ip: XdpIpAddress) -> anyhow::Result<Self> {
        let now = time_since_boot()?;
        let expire = Duration::from(now) + Duration::from_secs(EXPIRE_WATCHES_SECS);

        let mut enabled = 1;
        let mut map = BpfMap::<XdpIpAddress, u32>::from_path(HEIMDALL_WATCH_PATH)?;
        map.insert_or_update(&mut ip, &mut enabled).map_err(|err| {
            warn!("Unable to add Heimdall watch for {}: {err}", ip.as_ip());
            err
        })?;

        Ok(Self {
            ip_address: ip,
            expiration: expire.as_nanos(),
        })
    }

    fn stop_watching(&mut self) {
        info!(
            "Heimdall stopped watching {}",
            self.ip_address.as_ip().to_string()
        );
        let Ok(mut map) = BpfMap::<XdpIpAddress, u32>::from_path(HEIMDALL_WATCH_PATH) else {
            info!("Unable to access Heimdall map");
            return;
        };
        if let Err(err) = map.delete(&mut self.ip_address) {
            warn!(
                "Unable to remove Heimdall watch for {}: {err}",
                self.ip_address.as_ip()
            );
        }
    }
}

static HEIMDALL_WATCH_LIST: Lazy<DashMap<XdpIpAddress, HeimdallWatching>> = Lazy::new(DashMap::new);

/// Run this periodically (once per second) to expire any watched traffic
/// flows that haven't received traffic in the last 30 seconds.
pub fn heimdall_expire() {
    if let Ok(now) = time_since_boot() {
        let now = Duration::from(now).as_nanos();
        HEIMDALL_WATCH_LIST.retain(|_k, v| {
            if v.expiration < now {
                v.stop_watching();
            }
            v.expiration > now
        });
    }
}

/// Instruct Heimdall to start watching an IP address.
/// You want to call this when you refresh a flow; it will auto-expire
/// in 30 seconds.
pub fn heimdall_watch_ip(ip: XdpIpAddress) {
    if let Some(mut watch) = HEIMDALL_WATCH_LIST.get_mut(&ip) {
        if let Ok(now) = time_since_boot() {
            let expire = Duration::from(now) + Duration::from_secs(EXPIRE_WATCHES_SECS);
            watch.expiration = expire.as_nanos();
        }
    } else {
        match HeimdallWatching::new(ip) {
            Ok(h) => {
                debug!("Heimdall is watching {}", ip.as_ip());
                HEIMDALL_WATCH_LIST.insert(ip, h);
            }
            Err(err) => {
                warn!("Unable to start Heimdall watch for {}: {err}", ip.as_ip());
            }
        }
    }
}
