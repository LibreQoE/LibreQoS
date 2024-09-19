use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize};
use std::sync::mpsc::Sender;
use std::time::Duration;
use once_cell::sync::Lazy;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::debug;

const MAX_CPUS_COUNTED: usize = 128;

/// Stores overall CPU usage
pub static CPU_USAGE: Lazy<[AtomicU32; MAX_CPUS_COUNTED]> = Lazy::new(build_empty_cpu_list);

/// Total number of CPUs detected
pub static NUM_CPUS: AtomicUsize = AtomicUsize::new(0);

/// Total RAM used (bytes)
pub static RAM_USED: AtomicU64 = AtomicU64::new(0);

/// Total RAM installed (bytes)
pub static TOTAL_RAM: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct SystemStats {
    pub cpu_usage: Vec<u32>,
    pub ram_used: u64,
    pub total_ram: u64,
}

pub fn start_system_stats() -> anyhow::Result<Sender<tokio::sync::oneshot::Sender<SystemStats>>> {
    debug!("Starting system stats threads");
    let (tx, rx) = std::sync::mpsc::channel::<tokio::sync::oneshot::Sender<SystemStats>>();

    std::thread::Builder::new()
        .name("SysInfo Checker".to_string())
    .spawn(move || {
        // System Status Update Ticker Thread
        use sysinfo::System;
        let mut sys = System::new_all();

        // Timer
        let mut tfd = TimerFd::new().unwrap();
        assert_eq!(tfd.get_state(), TimerState::Disarmed);
        tfd.set_state(TimerState::Periodic{
            current: Duration::new(1, 0),
            interval: Duration::new(1, 0)}
                      , SetTimeFlags::Default
        );

        loop {
            let missed = tfd.read();
            if missed > 1 {
                debug!("System Stats Update: Missed {} ticks", missed);
            }

            sys.refresh_cpu_all();
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
    })?;

    std::thread::Builder::new()
        .name("SysInfo Channel".to_string())
    .spawn(move || {
        // Channel Receiver Thread
        while let Ok(sender) = rx.recv() {
            let mut cpus =CPU_USAGE.iter().map(|x| x.load(std::sync::atomic::Ordering::Relaxed)).collect::<Vec<u32>>();
            cpus.truncate(NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed));
            let cpu_usage = cpus;
            let ram_used = RAM_USED.load(std::sync::atomic::Ordering::Relaxed);
            let total_ram = TOTAL_RAM.load(std::sync::atomic::Ordering::Relaxed);

            let stats = SystemStats {
                cpu_usage,
                ram_used,
                total_ram,
            };

            // Ignoring error because it's just the data returned to the sender
            let _ = sender.send(stats);
        }
    })?;

    Ok(tx)
}

fn build_empty_cpu_list() -> [AtomicU32; MAX_CPUS_COUNTED] {
    let mut temp = Vec::with_capacity(MAX_CPUS_COUNTED);
    for _ in 0..MAX_CPUS_COUNTED {
        temp.push(AtomicU32::new(0));
    }
    temp.try_into().expect("This should never happen, sizes are constant.")
}
