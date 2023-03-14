#![warn(missing_docs)]

//! `lqos_sys` is a system-library that builds LibreQoS's eBPF code
//! and wraps it in a safe external Rust wrapper.
//!
//! The `build.rs` script compiles the C code found in `src/bpf`
//! and statically embeds the result in this crate.

mod bifrost_maps;
mod bpf_map;
mod bpf_per_cpu_map;
mod cpu_map;
mod heimdall_map;
mod ip_mapping;
mod kernel_wrapper;
mod lqos_kernel;
mod tcp_rtt;
mod throughput;
mod linux;

pub use heimdall_map::{
  heimdall_expire, heimdall_watch_ip, set_heimdall_mode
};
pub use ip_mapping::{
  add_ip_to_tc, clear_ips_from_tc, del_ip_from_tc, list_mapped_ips,
};
pub use kernel_wrapper::LibreQoSKernels;
pub use linux::num_possible_cpus;
pub use lqos_kernel::max_tracked_ips;
pub use tcp_rtt::{rtt_for_each, RttTrackingEntry};
pub use throughput::{throughput_for_each, HostCounter};
