use std::time::Duration;
use dashmap::DashMap;
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;

use crate::{bpf_per_cpu_map::BpfPerCpuMap, XdpIpAddress, bpf_map::BpfMap};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct HeimdallKey {
  pub src_ip: XdpIpAddress,
  pub dst_ip: XdpIpAddress,
  pub ip_protocol: u8,
  pub src_port: u16,
  pub dst_port: u16,
}

#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct HeimdallData {
  pub last_seen: u64,
  pub bytes: u64,
  pub packets: u64,
  pub tos: u8,
  pub reserved: [u8; 3],
}

/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
pub fn heimdall_for_each(
  callback: &mut dyn FnMut(&HeimdallKey, &[HeimdallData]),
) {
  if let Ok(heimdall) = BpfPerCpuMap::<HeimdallKey, HeimdallData>::from_path(
    "/sys/fs/bpf/heimdall",
  ) {
    heimdall.for_each(callback);
  }
}

#[repr(u8)]
pub enum HeimdallMode {
  Off = 0,
  WatchOnly = 1,
  Analysis = 2,
}

#[derive(Default, Clone)]
#[repr(C)]
struct HeimdalConfig {
  mode: u32,
}

/// Change the eBPF Heimdall System mode.
pub fn set_heimdall_mode(mode: HeimdallMode) -> anyhow::Result<()> {
  let mut map = BpfMap::<u32, HeimdalConfig>::from_path("/sys/fs/bpf/heimdall_config")?;
  map.clear_no_repeat()?;
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

pub fn heimdall_watch_ip(ip: XdpIpAddress) {
  if HEIMDALL_WATCH_LIST.contains_key(&ip) {
    return;
  }
  if let Ok(h) = HeimdallWatching::new(ip) {
    //println!("Watching {:?}", ip);
    HEIMDALL_WATCH_LIST.insert(ip, h);
  }
}