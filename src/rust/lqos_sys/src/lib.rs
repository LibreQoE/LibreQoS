#![warn(missing_docs)]

//! `lqos_sys` is a system-library that builds LibreQoS's eBPF code
//! and wraps it in a safe external Rust wrapper.
//!
//! The `build.rs` script compiles the C code found in `src/bpf`
//! and statically embeds the result in this crate.

mod bifrost_maps;
/// Provides direct access to LibBPF functionality, as exposed by the
/// built-in, compiled eBPF programs. This is very-low level and should
/// be handled with caution.
pub mod bpf_map;
mod cpu_map;
mod ip_mapping;
mod kernel_wrapper;
mod lqos_kernel;
mod throughput;
mod linux;
mod bpf_iterator;
/// Data shared between eBPF and Heimdall that needs local access
/// for map control.
pub mod flowbee_data;
mod garbage_collector;

pub use ip_mapping::{
  add_ip_to_tc, clear_ips_from_tc, del_ip_from_tc, list_mapped_ips, clear_hot_cache,
};
pub use kernel_wrapper::LibreQoSKernels;
pub use linux::num_possible_cpus;
pub use lqos_kernel::max_tracked_ips;
pub use throughput::{throughput_for_each, HostCounter};
pub use bpf_iterator::{iterate_flows, end_flows};
pub use lqos_kernel::interface_name_to_index;
pub use garbage_collector::bpf_garbage_collector;