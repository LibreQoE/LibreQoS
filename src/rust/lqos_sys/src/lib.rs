#![warn(missing_docs)]

//! `lqos_sys` retains the pure data structures that the bridge dataplane
//! still consumes. The XDP/TC kernel wrapper, its BPF maps, and the map
//! iterators were removed with the kernel datapath.

#![deny(clippy::unwrap_used)]

/// Data shared between the kernel and Heimdall that still needs local access.
pub mod flowbee_data;
mod linux;
mod throughput;

pub use linux::num_possible_cpus;
pub use throughput::HostCounter;
