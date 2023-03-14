use std::time::Duration;
use dashmap::DashMap;
use lqos_heimdall::{HeimdallMode, HeimdalConfig};
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use crate::{bpf_map::BpfMap};

/// Change the eBPF Heimdall System mode.
pub fn set_heimdall_mode(mode: HeimdallMode) -> anyhow::Result<()> {
  let mut map = BpfMap::<u32, HeimdalConfig>::from_path("/sys/fs/bpf/heimdall_config")?;
  map.insert_or_update(&mut 0, &mut HeimdalConfig { mode: mode as u32 })?;
  Ok(())
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct HeimdallWatching {
  expiration: u128,
  ip_address: XdpIpAddress
}

impl HeimdallWatching {
  pub fn new(mut ip: XdpIpAddress) -> anyhow::Result<Self> {
    let now = time_since_boot()?;
    let expire = Duration::from(now) + Duration::from_secs(30);

    let mut map = BpfMap::<XdpIpAddress, u32>::from_path("/sys/fs/bpf/heimdall_watching").unwrap();
    let _ = map.insert(&mut ip, &mut 1);

    Ok(Self {
      ip_address: ip,
      expiration: expire.as_nanos(),
    })
  }

  fn stop_watching(&mut self) {
    //println!("I stopped watching {:?}", self.ip_address);
    let mut map = BpfMap::<XdpIpAddress, u32>::from_path("/sys/fs/bpf/heimdall_watching").unwrap();
    map.delete(&mut self.ip_address).unwrap();
  }
}

static HEIMDALL_WATCH_LIST: Lazy<DashMap<XdpIpAddress, HeimdallWatching>> = Lazy::new(DashMap::new);

/// Run this periodically (once per second) to expire any watched traffic
/// flows that haven't received traffic in the last 30 seconds.
pub fn heimdall_expire() {
  if let Ok(now) = time_since_boot() {
    let now = Duration::from(now).as_nanos();
    HEIMDALL_WATCH_LIST.retain(|_k,v| {
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
  if HEIMDALL_WATCH_LIST.contains_key(&ip) {
    return;
  }
  if let Ok(h) = HeimdallWatching::new(ip) {
    //println!("Watching {:?}", ip);
    HEIMDALL_WATCH_LIST.insert(ip, h);
  }
}
