use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize};

const MAX_CPUS_COUNTED: usize = 128;

/// Stores overall CPU usage
pub static CPU_USAGE: Lazy<[AtomicU32; MAX_CPUS_COUNTED]> =
  Lazy::new(build_empty_cpu_list);

/// Total number of CPUs detected
pub static NUM_CPUS: AtomicUsize = AtomicUsize::new(0);

/// Total RAM used (bytes)
pub static RAM_USED: AtomicU64 = AtomicU64::new(0);

/// Total RAM installed (bytes)
pub static TOTAL_RAM: AtomicU64 = AtomicU64::new(0);

fn build_empty_cpu_list() -> [AtomicU32; MAX_CPUS_COUNTED] {
  let mut temp = Vec::with_capacity(MAX_CPUS_COUNTED);
  for _ in 0..MAX_CPUS_COUNTED {
    temp.push(AtomicU32::new(0));
  }
  temp.try_into().expect("This should never happen, sizes are constant.")
}