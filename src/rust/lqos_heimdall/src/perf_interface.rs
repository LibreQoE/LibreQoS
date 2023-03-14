use std::{ffi::c_void, slice};
use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

use crate::{flows::record_flow, timeline::store_on_timeline};

#[derive(FromBytes, Debug, Clone)]
#[repr(C)]
pub struct HeimdallEvent {
  pub timestamp: u64,
  pub src: XdpIpAddress,
  pub dst: XdpIpAddress,
  pub src_port : u16,
  pub dst_port: u16,
  pub ip_protocol: u8,
  pub tos: u8,
  pub size: u32,
}

/// Callback for the Heimdall Perf map system. Called whenever Heimdall has
/// events for the system to read.
///
/// # Safety
///
/// This function is inherently unsafe, because it interfaces directly with
/// C and the Linux-kernel eBPF system.
#[no_mangle]
pub unsafe extern "C" fn heimdall_handle_events(
  _ctx: *mut c_void,
  data: *mut c_void,
  data_size: usize,
) -> i32 {
  const EVENT_SIZE: usize = std::mem::size_of::<HeimdallEvent>();
  if data_size < EVENT_SIZE {
    log::warn!("Warning: incoming data too small in Heimdall buffer");
    return 0;
  }

  //COLLECTED_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  let data_u8 = data as *const u8;
  let data_slice : &[u8] = slice::from_raw_parts(data_u8, EVENT_SIZE);

  if let Some(incoming) = HeimdallEvent::read_from(data_slice) {
    record_flow(&incoming);
    store_on_timeline(incoming);
  } else {
    println!("Failed to decode");
  }

  0
}
