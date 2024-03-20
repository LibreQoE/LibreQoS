//! Provides a sysinfo link for CPU and RAM tracking

use crate::ui_base::SHOULD_EXIT;
use once_cell::sync::Lazy;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize};

const MAX_CPUS_COUNTED: usize = 128;

/// Stores overall CPU usage
pub static CPU_USAGE: Lazy<[AtomicU32; MAX_CPUS_COUNTED]> = Lazy::new(build_empty_cpu_list);

/// Total number of CPUs detected
pub static NUM_CPUS: AtomicUsize = AtomicUsize::new(0);

/// Total RAM used (bytes)
pub static RAM_USED: AtomicU64 = AtomicU64::new(0);

/// Total RAM installed (bytes)
pub static TOTAL_RAM: AtomicU64 = AtomicU64::new(0);
pub async fn gather_sysinfo() {
    use sysinfo::System;
    let mut sys = System::new_all();

    loop {
        if SHOULD_EXIT.load(Ordering::Relaxed) {
            break;
        }

        // Refresh system info
        sys.refresh_cpu();
        sys.refresh_memory();

        sys.cpus()
            .iter()
            .enumerate()
            .map(|(i, cpu)| (i, cpu.cpu_usage() as u32)) // Always rounds down
            .for_each(|(i, cpu)| CPU_USAGE[i].store(cpu, std::sync::atomic::Ordering::Relaxed));

        NUM_CPUS.store(sys.cpus().len(), std::sync::atomic::Ordering::Relaxed);
        RAM_USED.store(sys.used_memory(), std::sync::atomic::Ordering::Relaxed);
        TOTAL_RAM.store(sys.total_memory(), std::sync::atomic::Ordering::Relaxed);

        // Sleep
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

fn build_empty_cpu_list() -> [AtomicU32; MAX_CPUS_COUNTED] {
    let mut temp = Vec::with_capacity(MAX_CPUS_COUNTED);
    for _ in 0..MAX_CPUS_COUNTED {
        temp.push(AtomicU32::new(0));
    }
    temp.try_into()
        .expect("This should never happen, sizes are constant.")
}
