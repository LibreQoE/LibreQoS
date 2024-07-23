use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize};
use std::time::Duration;

use once_cell::sync::Lazy;

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

pub async fn update_cache() {
    use sysinfo::System;
    let mut sys = System::new_all();
    tokio::time::sleep(Duration::from_secs(10)).await;

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    loop {
        interval.tick().await; // Once per second

        sys.refresh_cpu();
        sys.refresh_memory();

        sys
            .cpus()
            .iter()
            .enumerate()
            .map(|(i, cpu)| (i, cpu.cpu_usage() as u32)) // Always rounds down
            .for_each(|(i, cpu)| {
                CPU_USAGE[i].store(cpu, std::sync::atomic::Ordering::Relaxed)
            });

        NUM_CPUS
            .store(sys.cpus().len(), std::sync::atomic::Ordering::Relaxed);
        RAM_USED
            .store(sys.used_memory(), std::sync::atomic::Ordering::Relaxed);
        TOTAL_RAM
            .store(sys.total_memory(), std::sync::atomic::Ordering::Relaxed);
    }
}