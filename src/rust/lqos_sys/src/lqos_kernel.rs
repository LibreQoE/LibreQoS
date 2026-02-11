#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use crate::cpu_map::CpuMapping;
use anyhow::{Error, Result};
use libbpf_sys::{
    LIBBPF_STRICT_ALL, XDP_FLAGS_DRV_MODE, XDP_FLAGS_HW_MODE, XDP_FLAGS_SKB_MODE,
    XDP_FLAGS_UPDATE_IF_NOEXIST, bpf_map_info, bpf_obj_get, bpf_obj_get_info_by_fd, bpf_xdp_attach,
    libbpf_set_strict_mode,
};
use lqos_utils::XdpIpAddress;
use nix::libc::{close, geteuid, if_nametoindex};
use std::{
    ffi::{CString, c_void},
    fs,
    mem::MaybeUninit,
    path::Path,
    process::Command,
    thread,
    time::Duration,
};
use tracing::{debug, error, info, warn};

use self::bpf::{libbpf_num_possible_cpus, lqos_kern};

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

fn remove_incompatible_pinned_map(
    path: &str,
    expected_key_size: u32,
    expected_value_size: u32,
) -> Result<()> {
    if !Path::new(path).exists() {
        return Ok(());
    }

    let path_c = CString::new(path)?;
    let fd = unsafe { bpf_obj_get(path_c.as_ptr()) };
    if fd < 0 {
        warn!("Unable to open pinned BPF map '{path}' for ABI check (fd={fd})");
        return Ok(());
    }

    let mut info = MaybeUninit::<bpf_map_info>::zeroed();
    let mut len: u32 = std::mem::size_of::<bpf_map_info>() as u32;
    let err = unsafe { bpf_obj_get_info_by_fd(fd, info.as_mut_ptr() as *mut c_void, &mut len) };
    unsafe {
        close(fd);
    }
    if err != 0 {
        return Err(Error::msg(format!(
            "Unable to query pinned BPF map '{path}' info (err={err})."
        )));
    }
    let info = unsafe { info.assume_init() };

    if info.key_size != expected_key_size || info.value_size != expected_value_size {
        warn!(
            "Pinned BPF map '{}' ABI mismatch (key_size={}, value_size={}) expected (key_size={}, value_size={}). Removing pin to force recreation.",
            path, info.key_size, info.value_size, expected_key_size, expected_value_size
        );
        fs::remove_file(path).map_err(|e| {
            Error::msg(format!(
                "Unable to remove pinned BPF map '{path}' (needed for ABI upgrade): {e:?}"
            ))
        })?;
    }

    Ok(())
}

fn ensure_ip_mapping_maps_abi() -> Result<()> {
    let expected_key_size = std::mem::size_of::<crate::ip_mapping::IpHashKey>() as u32;
    let expected_value_size = std::mem::size_of::<crate::ip_mapping::IpHashData>() as u32;
    remove_incompatible_pinned_map(
        "/sys/fs/bpf/map_ip_to_cpu_and_tc",
        expected_key_size,
        expected_value_size,
    )?;
    remove_incompatible_pinned_map(
        "/sys/fs/bpf/ip_to_cpu_and_tc_hotcache",
        std::mem::size_of::<XdpIpAddress>() as u32,
        expected_value_size,
    )?;
    remove_incompatible_pinned_map(
        "/sys/fs/bpf/ip_mapping_epoch",
        std::mem::size_of::<u32>() as u32,
        std::mem::size_of::<u32>() as u32,
    )?;
    remove_incompatible_pinned_map(
        "/sys/fs/bpf/map_traffic",
        std::mem::size_of::<XdpIpAddress>() as u32,
        std::mem::size_of::<crate::HostCounter>() as u32,
    )?;
    remove_incompatible_pinned_map(
        "/sys/fs/bpf/flowbee",
        std::mem::size_of::<crate::flowbee_data::FlowbeeKey>() as u32,
        std::mem::size_of::<crate::flowbee_data::FlowbeeData>() as u32,
    )?;
    Ok(())
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

/// Removes the XDP bindings from an interface.
///
/// # Arguments
/// * `interface_name` - The name of the interface from which you wish to remove XDP
pub fn unload_xdp_from_interface(interface_name: &str) -> Result<()> {
    debug!("Unloading XDP/TC on {}", interface_name);
    check_root()?;
    let ifindex_u32: u32 = interface_name_to_index(interface_name)?;
    let ifindex_i32: i32 = ifindex_u32.try_into()?;

    // Loop: aggressively attempt detaches across all modes a few times
    let modes = [
        XDP_FLAGS_HW_MODE,
        XDP_FLAGS_DRV_MODE,
        XDP_FLAGS_SKB_MODE,
        0u32,
    ];
    for _ in 0..10 {
        let mut any_success = false;
        for flags in modes {
            let err = unsafe { bpf_xdp_attach(ifindex_i32, -1, flags, std::ptr::null()) };
            if err == 0 {
                any_success = true;
                let mode = match flags {
                    x if x == XDP_FLAGS_HW_MODE => "HW",
                    x if x == XDP_FLAGS_DRV_MODE => "DRV",
                    x if x == XDP_FLAGS_SKB_MODE => "SKB",
                    _ => "DEFAULT",
                };
                debug!("Detached XDP on {} (mode {})", interface_name, mode);
            }
        }
        if !any_success {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    // As a last resort, ask ip(8) to turn off XDP in all modes
    let _ = Command::new("/bin/ip")
        .args(["link", "set", interface_name, "xdp", "off"])
        .output();
    let _ = Command::new("/bin/ip")
        .args(["link", "set", interface_name, "xdpdrv", "off"])
        .output();
    let _ = Command::new("/bin/ip")
        .args(["link", "set", interface_name, "xdpgeneric", "off"])
        .output();

    // Detach TC hooks as well
    unsafe {
        let interface_c = CString::new(interface_name)?;
        let _ = bpf::tc_detach_egress(ifindex_i32, false, true, interface_c.as_ptr());
        let _ = bpf::tc_detach_ingress(ifindex_i32, false, true, interface_c.as_ptr());
    }

    Ok(())
}

fn set_strict_mode() -> Result<()> {
    let err = unsafe { libbpf_set_strict_mode(LIBBPF_STRICT_ALL) };
    #[cfg(not(debug_assertions))]
    unsafe {
        bpf::do_not_print();
    }
    if err != 0 {
        Err(Error::msg("Unable to activate BPF Strict Mode"))
    } else {
        Ok(())
    }
}

/// Safety: This function is unsafe because it is directly calling into the
/// C library to open the kernel.
unsafe fn open_kernel() -> Result<*mut bpf::lqos_kern> {
    let result = unsafe { bpf::lqos_kern_open() };
    if result.is_null() {
        Err(Error::msg("Unable to open LibreQoS XDP/TC Kernel"))
    } else {
        Ok(result)
    }
}

unsafe fn load_kernel(skeleton: *mut bpf::lqos_kern) -> Result<()> {
    let error = unsafe { bpf::lqos_kern_load(skeleton) };
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
    OnAStick(u16, u16, u32),
}

pub fn attach_xdp_and_tc_to_interface(
    interface_name: &str,
    direction: InterfaceDirection,
    heimdall_event_handler: bpf::ring_buffer_sample_fn,
    flowbee_event_handler: bpf::ring_buffer_sample_fn,
) -> Result<*mut lqos_kern> {
    check_root()?;
    // If ABI changes were made to pinned maps, ensure we do not silently reuse
    // incompatible versions that truncate struct values.
    ensure_ip_mapping_maps_abi()?;
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
        if let InterfaceDirection::OnAStick(internet, isp, stick_offset) = direction {
            (*(*skeleton).bss).internet_vlan = internet.to_be();
            (*(*skeleton).bss).isp_vlan = isp.to_be();
            (*(*skeleton).bss).stick_offset = stick_offset;
        }
        // Ensure no lingering XDP programs before loading/attaching
        let _ = unload_xdp_from_interface(interface_name);
        load_kernel(skeleton)?;
        let prog_fd = bpf::bpf_program__fd((*skeleton).progs.xdp_prog);
        attach_xdp_best_available(interface_index, prog_fd, interface_name)?;
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
    let _ =
        unsafe { bpf::tc_detach_egress(interface_index as i32, false, true, interface_c.as_ptr()) }; // Ignoring error, because it's ok to not have something to detach

    // Find the heimdall_events perf map by name
    let heimdall_events_name = c"heimdall_events";
    let heimdall_events_map = unsafe {
        bpf::bpf_object__find_map_by_name((*skeleton).obj, heimdall_events_name.as_ptr())
    };
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
            opts as *mut c_void,
            opts,
        )
    };
    if unsafe { bpf::libbpf_get_error(heimdall_perf_buffer as *mut c_void) != 0 } {
        error!("Failed to create Heimdall event buffer");
        return Err(anyhow::Error::msg("Failed to create Heimdall event buffer"));
    }
    let handle = PerfBufferHandle(heimdall_perf_buffer);
    std::thread::Builder::new()
        .name("HeimdallEvents".to_string())
        .spawn(|| poll_perf_events(handle))?;

    // Find and attach the Flowbee handler
    let flowbee_events_name = c"flowbee_events";
    let flowbee_events_map =
        unsafe { bpf::bpf_object__find_map_by_name((*skeleton).obj, flowbee_events_name.as_ptr()) };
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
            opts as *mut c_void,
            opts,
        )
    };
    if unsafe { bpf::libbpf_get_error(flowbee_perf_buffer as *mut c_void) != 0 } {
        error!("Failed to create Flowbee event buffer");
        return Err(anyhow::Error::msg("Failed to create Flowbee event buffer"));
    }
    let handle = PerfBufferHandle(flowbee_perf_buffer);
    std::thread::Builder::new()
        .name(format!("FlowEvents_{}", interface_name))
        .spawn(|| poll_perf_events(handle))?;

    // Remove any previous entry
    let _r = Command::new("tc")
        .args(["qdisc", "del", "dev", interface_name, "clsact"])
        .output()?;
    // This message was worrying people, commented out.
    //println!("{}", String::from_utf8(r.stderr).unwrap());

    // Ensure clsact qdisc exists (libbpf APIs will create hooks, but this makes state explicit)
    let _r = Command::new("tc")
        .args(["qdisc", "add", "dev", interface_name, "clsact"])
        .output()?;
    // This message was worrying people, commented out.
    //println!("{}", String::from_utf8(r.stderr).unwrap());

    // Attach to the egress
    let error = unsafe { bpf::tc_attach_egress(interface_index as i32, false, skeleton) };
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
                crate::bifrost_maps::map_multi_interface_mode(
                    &bridge.to_internet,
                    &bridge.to_network,
                )?;

                // Actually attach the TC ingress program
                let error =
                    unsafe { bpf::tc_attach_ingress(interface_index as i32, false, skeleton) };
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
            crate::bifrost_maps::map_single_interface_mode(
                &stick.interface,
                stick.internet_vlan as u32,
                stick.network_vlan as u32,
            )?;

            // Actually attach the TC ingress program
            let error = unsafe { bpf::tc_attach_ingress(interface_index as i32, false, skeleton) };
            if error != 0 {
                return Err(Error::msg("Unable to attach TC Ingress to interface"));
            }
        }
    }

    Ok(skeleton)
}

/// Safety: Direct calls to C functions
unsafe fn attach_xdp_best_available(
    interface_index: u32,
    prog_fd: i32,
    iface_name: &str,
) -> Result<()> {
    // Helper: attempt attach for a mode with limited retries on EBUSY/EEXIST
    fn should_retry(errno: i32) -> bool {
        errno == -16 || errno == -17
    }

    /// Safety: Direct calls to C functions
    unsafe fn try_mode_with_retries(
        iface_index: u32,
        prog_fd: i32,
        mode_flag: Option<u32>,
        iface_name: &str,
        max_retries: usize,
    ) -> Result<(), i32> {
        let mut attempts = 0;
        loop {
            let err = match mode_flag {
                Some(flag) => unsafe {
                    bpf_xdp_attach(
                        iface_index.try_into().expect("Invalid interface index"),
                        prog_fd,
                        XDP_FLAGS_UPDATE_IF_NOEXIST | flag,
                        std::ptr::null(),
                    )
                },
                None => unsafe {
                    bpf_xdp_attach(
                        iface_index.try_into().expect("Invalid interface index"),
                        prog_fd,
                        XDP_FLAGS_UPDATE_IF_NOEXIST,
                        std::ptr::null(),
                    )
                },
            };
            if err == 0 {
                return Ok(());
            }
            if should_retry(err) && attempts < max_retries {
                // Proactively detach any lingering XDP and retry
                let _ = unload_xdp_from_interface(iface_name);
                thread::sleep(Duration::from_millis(50));
                attempts += 1;
                continue;
            }
            return Err(err);
        }
    }

    // Try hardware offload first
    match unsafe {
        try_mode_with_retries(
            interface_index,
            prog_fd,
            Some(XDP_FLAGS_HW_MODE),
            iface_name,
            2,
        )
    } {
        Ok(()) => {
            info!(
                "Attached '{}' in hardware accelerated mode. (Fastest)",
                iface_name
            );
            return Ok(());
        }
        Err(_e_hw) => {
            debug!(
                "XDP attach in HW mode failed (errno: {}), falling back",
                _e_hw
            );
        }
    }

    // Try driver attach
    match unsafe {
        try_mode_with_retries(
            interface_index,
            prog_fd,
            Some(XDP_FLAGS_DRV_MODE),
            iface_name,
            5,
        )
    } {
        Ok(()) => {
            info!("Attached '{}' in driver mode. (Fast)", iface_name);
            return Ok(());
        }
        Err(e_drv) => {
            debug!(
                "XDP attach in DRV mode failed (errno: {}), falling back",
                e_drv
            );
        }
    }

    // Try SKB mode
    match unsafe {
        try_mode_with_retries(
            interface_index,
            prog_fd,
            Some(XDP_FLAGS_SKB_MODE),
            iface_name,
            5,
        )
    } {
        Ok(()) => {
            info!(
                "Attached '{}' in SKB compatibility mode. (Not so fast)",
                iface_name
            );
            return Ok(());
        }
        Err(e_skb) => {
            debug!(
                "XDP attach in SKB mode failed (errno: {}), falling back",
                e_skb
            );
        }
    }

    // Try no flags
    match unsafe { try_mode_with_retries(interface_index, prog_fd, None, iface_name, 3) } {
        Ok(()) => return Ok(()),
        Err(error) => {
            error!(
                "XDP attach failed on '{}' in all modes (errno: {}). Suggestion: check for existing XDP programs (ip link show, bpftool net), detach with 'ip link set dev {} xdp off', and clear pinned maps if needed.",
                iface_name, error, iface_name
            );
            return Err(Error::msg("Unable to attach to interface"));
        }
    }
}

// (removed) try_xdp_attach: replaced by try_mode_with_retries inside attach_xdp_best_available

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
