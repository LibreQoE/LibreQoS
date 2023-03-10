use crate::{bpf_per_cpu_map::BpfPerCpuMap, XdpIpAddress};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct PalantirKey {
  pub src_ip: XdpIpAddress,
  pub dst_ip: XdpIpAddress,
  pub ip_protocol: u8,
  pub src_port: u16,
  pub dst_port: u16,
}

#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct PalantirData {
  pub last_seen: u64,
  pub bytes: u64,
  pub packets: u64,
  pub tos: u8,
  pub reserved: [u8; 3],
}

/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
pub fn palantir_for_each(
  callback: &mut dyn FnMut(&PalantirKey, &[PalantirData]),
) {
  if let Ok(palantir) = BpfPerCpuMap::<PalantirKey, PalantirData>::from_path(
    "/sys/fs/bpf/palantir",
  ) {
    palantir.for_each(callback);
  }
}
