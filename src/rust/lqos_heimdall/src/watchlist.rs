use crate::{HeimdalConfig, HeimdallMode, EXPIRE_WATCHES_SECS};
use dashmap::DashMap;
use lqos_sys::bpf_map::BpfMap;
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use std::time::Duration;

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
    let expire =
      Duration::from(now) + Duration::from_secs(EXPIRE_WATCHES_SECS);

    let mut map =
      BpfMap::<XdpIpAddress, u32>::from_path(HEIMDALL_WATCH_PATH).unwrap();
    let _ = map.insert(&mut ip, &mut 1);

    Ok(Self { ip_address: ip, expiration: expire.as_nanos() })
  }

  fn stop_watching(&mut self) {
    log::info!("Heimdall stopped watching {}", self.ip_address.as_ip().to_string());
    let mut map =
      BpfMap::<XdpIpAddress, u32>::from_path(HEIMDALL_WATCH_PATH).unwrap();
    map.delete(&mut self.ip_address).unwrap();
  }
}

impl Drop for HeimdallWatching {
  fn drop(&mut self) {
      self.stop_watching();
  }
}

static HEIMDALL_WATCH_LIST: Lazy<DashMap<XdpIpAddress, HeimdallWatching>> =
  Lazy::new(DashMap::new);

/// Run this periodically (once per second) to expire any watched traffic
/// flows that haven't received traffic in the last 30 seconds.
pub fn heimdall_expire() {
  if let Ok(now) = time_since_boot() {
    let now = Duration::from(now).as_nanos();
    HEIMDALL_WATCH_LIST.retain(|_k, v| {
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
      let expire =
        Duration::from(now) + Duration::from_secs(EXPIRE_WATCHES_SECS);
      watch.expiration = expire.as_nanos();
    }
  } else if let Ok(h) = HeimdallWatching::new(ip) {
    log::info!("Heimdall is watching {}", ip.as_ip().to_string());
    HEIMDALL_WATCH_LIST.insert(ip, h);
  }
}
