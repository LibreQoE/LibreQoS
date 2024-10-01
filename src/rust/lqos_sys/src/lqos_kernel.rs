#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use crate::cpu_map::CpuMapping;
use anyhow::{Error, Result};
use libbpf_sys::{
  bpf_xdp_attach, libbpf_set_strict_mode, LIBBPF_STRICT_ALL,
  XDP_FLAGS_DRV_MODE, XDP_FLAGS_HW_MODE, XDP_FLAGS_SKB_MODE,
  XDP_FLAGS_UPDATE_IF_NOEXIST,
};
use tracing::{debug, error, info, warn};
use nix::libc::{geteuid, if_nametoindex};
use std::{ffi::{CString, c_void}, process::Command};

use self::bpf::{lqos_kern, libbpf_num_possible_cpus};

pub(crate) mod bpf {
  #![allow(warnings, unused)]
  include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Returns the value set in the C XDP system's MAX_TRACKED_IPS
/// constant.
pub fn max_tracked_ips() -> usize {
  (unsafe { bpf::max_tracker_ips() }) as usize
}

pub fn check_root() -> Result<()> {
  unsafe {
    if geteuid() == 0 {
      Ok(())
    } else {
      Err(Error::msg("You need to be root to do this."))
    }
  }
}

/// Converts an interface name to an interface index.
/// This is a wrapper around the `if_nametoindex` function.
/// Returns an error if the interface does not exist.
/// # Arguments
/// * `interface_name` - The name of the interface to convert
/// # Returns
/// * The index of the interface
pub fn interface_name_to_index(interface_name: &str) -> Result<u32> {
  let if_name = CString::new(interface_name)?;
  let index = unsafe { if_nametoindex(if_name.as_ptr()) };
  if index == 0 {
    Err(Error::msg(format!("Unknown interface: {interface_name}")))
  } else {
    Ok(index)
  }
}

pub fn unload_xdp_from_interface(interface_name: &str) -> Result<()> {
  debug!("Unloading XDP/TC");
  check_root()?;
  let interface_index = interface_name_to_index(interface_name)?.try_into()?;
  unsafe {
    let err = bpf_xdp_attach(interface_index, -1, 1 << 0, std::ptr::null());
    if err != 0 {
      return Err(Error::msg("Unable to unload from interface."));
    }

    let interface_c = CString::new(interface_name)?;
    let _ = bpf::tc_detach_egress(
      interface_index,
      false,
      true,
      interface_c.as_ptr(),
    );
    let _ = bpf::tc_detach_ingress(
      interface_index,
      false,
      true,
      interface_c.as_ptr(),
    );
  }
  Ok(())
}

fn set_strict_mode() -> Result<()> {
  let err = unsafe { libbpf_set_strict_mode(LIBBPF_STRICT_ALL) };
  //#[cfg(not(debug_assertions))]
  unsafe {
    bpf::do_not_print();
  }
  if err != 0 {
    Err(Error::msg("Unable to activate BPF Strict Mode"))
  } else {
    Ok(())
  }
}

unsafe fn open_kernel() -> Result<*mut bpf::lqos_kern> {
  let result = bpf::lqos_kern_open();
  if result.is_null() {
    Err(Error::msg("Unable to open LibreQoS XDP/TC Kernel"))
  } else {
    Ok(result)
  }
}

unsafe fn load_kernel(skeleton: *mut bpf::lqos_kern) -> Result<()> {
  let error = bpf::lqos_kern_load(skeleton);
  if error != 0 {
    let error = format!("Unable to load the XDP/TC kernel ({error})");
    Err(Error::msg(error))
  } else {
    Ok(())
  }
}

#[derive(PartialEq, Eq)]
pub enum InterfaceDirection {
  Internet,
  IspNetwork,
  OnAStick(u16, u16),
}

pub fn attach_xdp_and_tc_to_interface(
  interface_name: &str,
  direction: InterfaceDirection,
  heimdall_event_handler: bpf::ring_buffer_sample_fn,
  flowbee_event_handler: bpf::ring_buffer_sample_fn,
) -> Result<*mut lqos_kern> {
  check_root()?;
  // Check the interface is valid
  let interface_index = interface_name_to_index(interface_name)?;
  set_strict_mode()?;
  let skeleton = unsafe {
    let skeleton = open_kernel()?;
    (*(*skeleton).rodata).NUM_CPUS = libbpf_num_possible_cpus();
    (*(*skeleton).data).direction = match direction {
      InterfaceDirection::Internet => 1,
      InterfaceDirection::IspNetwork => 2,
      InterfaceDirection::OnAStick(..) => 3,
    };
    if let InterfaceDirection::OnAStick(internet, isp) = direction {
      (*(*skeleton).bss).internet_vlan = internet.to_be();
      (*(*skeleton).bss).isp_vlan = isp.to_be();
    }
    load_kernel(skeleton)?;
    let _ = unload_xdp_from_interface(interface_name); // Ignoring error, it's ok if there isn't one
    let prog_fd = bpf::bpf_program__fd((*skeleton).progs.xdp_prog);
    attach_xdp_best_available(interface_index, prog_fd)?;
    skeleton
  };

  // Configure CPU Maps
  {
    let cpu_map = CpuMapping::new()?;
    crate::cpu_map::xps_setup_default_disable(interface_name)?;
    cpu_map.mark_cpus_available()?;
    cpu_map.setup_base_txq_config()?;
  } // Scope block to ensure the CPU maps are closed

  // Attach the TC program
  // extern int tc_attach_egress(int ifindex, bool verbose, struct lqos_kern *obj);
  // extern int tc_detach_egress(int ifindex, bool verbose, bool flush_hook, char * ifname);
  let interface_c = CString::new(interface_name)?;
  let _ = unsafe {
    bpf::tc_detach_egress(
      interface_index as i32,
      false,
      true,
      interface_c.as_ptr(),
    )
  }; // Ignoring error, because it's ok to not have something to detach

  // Find the heimdall_events perf map by name
  let heimdall_events_name = CString::new("heimdall_events").unwrap();
  let heimdall_events_map = unsafe { bpf::bpf_object__find_map_by_name((*skeleton).obj, heimdall_events_name.as_ptr()) };
  let heimdall_events_fd = unsafe { bpf::bpf_map__fd(heimdall_events_map) };
  if heimdall_events_fd < 0 {
    error!("Unable to load Heimdall Events FD");
    return Err(anyhow::Error::msg("Unable to load Heimdall Events FD"));
  }
  let opts: *const bpf::ring_buffer_opts = std::ptr::null();
  let heimdall_perf_buffer = unsafe {
    bpf::ring_buffer__new(
      heimdall_events_fd, 
      heimdall_event_handler, 
      opts as *mut c_void, opts)
  };
  if unsafe { bpf::libbpf_get_error(heimdall_perf_buffer as *mut c_void) != 0 } {
    error!("Failed to create Heimdall event buffer");
    return Err(anyhow::Error::msg("Failed to create Heimdall event buffer"));
  }
  let handle = PerfBufferHandle(heimdall_perf_buffer);
  std::thread::Builder::new().name("HeimdallEvents".to_string()).spawn(|| poll_perf_events(handle))?;

  // Find and attach the Flowbee handler
  let flowbee_events_name = CString::new("flowbee_events").unwrap();
  let flowbee_events_map = unsafe { bpf::bpf_object__find_map_by_name((*skeleton).obj, flowbee_events_name.as_ptr()) };
  let flowbee_events_fd = unsafe { bpf::bpf_map__fd(flowbee_events_map) };
  if flowbee_events_fd < 0 {
    error!("Unable to load Flowbee Events FD");
    return Err(anyhow::Error::msg("Unable to load Flowbee Events FD"));
  }
  let opts: *const bpf::ring_buffer_opts = std::ptr::null();
  let flowbee_perf_buffer = unsafe {
    bpf::ring_buffer__new(
      flowbee_events_fd, 
      flowbee_event_handler, 
      opts as *mut c_void, opts)
  };
  if unsafe { bpf::libbpf_get_error(flowbee_perf_buffer as *mut c_void) != 0 } {
    error!("Failed to create Flowbee event buffer");
    return Err(anyhow::Error::msg("Failed to create Flowbee event buffer"));
  }
  let handle = PerfBufferHandle(flowbee_perf_buffer);
  std::thread::Builder::new().name("FlowEvents".to_string()).spawn(|| poll_perf_events(handle))?;

  // Remove any previous entry
  let _r = Command::new("tc")
    .args(["qdisc", "del", "dev", interface_name, "clsact"])
    .output()?;
  // This message was worrying people, commented out.
  //println!("{}", String::from_utf8(r.stderr).unwrap());

  // Add the classifier
  let _r = Command::new("tc")
    .args(["filter", "add", "dev", interface_name, "clsact"])
    .output()?;
  // This message was worrying people, commented out.
  //println!("{}", String::from_utf8(r.stderr).unwrap());

  // Attach to the egress
  let error =
    unsafe { bpf::tc_attach_egress(interface_index as i32, false, skeleton) };
  if error != 0 {
    return Err(Error::msg("Unable to attach TC to interface"));
  }

  // Attach to the ingress IF it is configured
  if let Ok(etc) = lqos_config::load_config() {
    if let Some(bridge) = &etc.bridge {
      if bridge.use_xdp_bridge {
        // Enable "promiscuous" mode on interfaces
        debug!("Enabling promiscuous mode on {}", &bridge.to_internet);
        std::process::Command::new("/bin/ip")
              .args(["link", "set", &bridge.to_internet, "promisc", "on"])
              .output()?;
        debug!("Enabling promiscuous mode on {}", &bridge.to_network);
        std::process::Command::new("/bin/ip")
          .args(["link", "set", &bridge.to_network, "promisc", "on"])
          .output()?;

        // Build the interface and vlan map entries
        crate::bifrost_maps::clear_bifrost()?;
        crate::bifrost_maps::map_multi_interface_mode(&bridge.to_internet, &bridge.to_network)?;

        // Actually attach the TC ingress program
        let error = unsafe {
          bpf::tc_attach_ingress(interface_index as i32, false, skeleton)
        };
        if error != 0 {
          return Err(Error::msg("Unable to attach TC Ingress to interface"));
        }
      }
    }

    if let Some(stick) = &etc.single_interface {
      // Enable "promiscuous" mode on interface
      debug!("Enabling promiscuous mode on {}", &stick.interface);
      std::process::Command::new("/bin/ip")
        .args(["link", "set", &stick.interface, "promisc", "on"])
        .output()?;

      // Build the interface and vlan map entries
      crate::bifrost_maps::clear_bifrost()?;
      crate::bifrost_maps::map_single_interface_mode(&stick.interface, stick.internet_vlan as u32, stick.network_vlan as u32)?;

      // Actually attach the TC ingress program
      let error = unsafe {
        bpf::tc_attach_ingress(interface_index as i32, false, skeleton)
      };
      if error != 0 {
        return Err(Error::msg("Unable to attach TC Ingress to interface"));
      }
    }
  }

  
  Ok(skeleton)
}

unsafe fn attach_xdp_best_available(
  interface_index: u32,
  prog_fd: i32,
) -> Result<()> {
  // Try hardware offload first
  if try_xdp_attach(interface_index, prog_fd, XDP_FLAGS_HW_MODE).is_err() {
    // Try driver attach
    if try_xdp_attach(interface_index, prog_fd, XDP_FLAGS_DRV_MODE).is_err() {
      // Try SKB mode
      if try_xdp_attach(interface_index, prog_fd, XDP_FLAGS_SKB_MODE).is_err()
      {
        // Try no flags
        let error = bpf_xdp_attach(
          interface_index.try_into().unwrap(),
          prog_fd,
          XDP_FLAGS_UPDATE_IF_NOEXIST,
          std::ptr::null(),
        );
        if error != 0 {
          return Err(Error::msg("Unable to attach to interface"));
        }
      } else {
        warn!("Attached in SKB compatibility mode. (Not so fast)");
      }
    } else {
      info!("Attached in driver mode. (Fast)");
    }
  } else {
    info!("Attached in hardware accelerated mode. (Fastest)");
  }
  Ok(())
}

unsafe fn try_xdp_attach(
  interface_index: u32,
  prog_fd: i32,
  connect_mode: u32,
) -> Result<()> {
  let error = bpf_xdp_attach(
    interface_index.try_into().unwrap(),
    prog_fd,
    XDP_FLAGS_UPDATE_IF_NOEXIST | connect_mode,
    std::ptr::null(),
  );
  if error != 0 {
    return Err(Error::msg("Unable to attach to interface"));
  }
  Ok(())
}

// Handle type used to wrap *mut bpf::perf_buffer and indicate
// that it can be moved. Really unsafe code in theory.
struct PerfBufferHandle(*mut bpf::ring_buffer);
unsafe impl Send for PerfBufferHandle {}
unsafe impl Sync for PerfBufferHandle {}

/// Run this in a thread, or doom will surely hit you
fn poll_perf_events(heimdall_perf_buffer: PerfBufferHandle) {
  let heimdall_perf_buffer = heimdall_perf_buffer.0;
  loop {
    let err = unsafe { bpf::ring_buffer__poll(heimdall_perf_buffer, 100) };
    if err < 0 {
      error!("Error polling perfbuffer");
    }
  }
}
