//! The cache module stores cached data, periodically
//! obtained from the `lqosd` server and other parts
//! of the system.

mod cpu_ram;
mod shaped_devices;
mod throughput;
mod dns_cache;

pub use cpu_ram::*;
pub use shaped_devices::*;
pub use throughput::THROUGHPUT_BUFFER;
pub use dns_cache::lookup_dns;
