//! Memory pressure diagnostics for `lqosd`.
//!
//! The watchdog samples `/proc` and logs host and process memory pressure. It
//! does not terminate the daemon; the logs are intended to preserve useful
//! diagnostic context while leaving restart decisions to system policy and
//! operators.

use crate::stats::{BUS_REQUESTS, HIGH_WATERMARK, TIME_TO_POLL_HOSTS};
use crate::throughput_tracker::flow_data::live_active_flow_count;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::{error, info, warn};

const CHECK_INTERVAL_SECONDS: u64 = 15;
const HOST_PRESSURE_AVAILABLE_PERCENT: u64 = 10;
const PROCESS_CRITICAL_TOTAL_RAM_PERCENT: u64 = 90;

/// Spawns the memory watchdog thread.
///
/// Side effects: this function starts a background thread. The thread reads
/// `/proc/meminfo` and `/proc/self/status` to log memory pressure diagnostics.
pub fn start_memory_watchdog() {
    if env_flag_enabled("LQOSD_MEMORY_WATCHDOG_DISABLED") {
        warn!("lqosd memory watchdog disabled by LQOSD_MEMORY_WATCHDOG_DISABLED");
        return;
    }

    match std::thread::Builder::new()
        .name("Memory Watchdog".to_string())
        .spawn(memory_watchdog_loop)
    {
        Ok(_) => info!("lqosd memory watchdog started"),
        Err(err) => warn!("Failed to start lqosd memory watchdog: {err:?}"),
    }
}

fn memory_watchdog_loop() {
    let mut state = MemoryWatchdogState::default();

    loop {
        std::thread::sleep(Duration::from_secs(CHECK_INTERVAL_SECONDS));

        let Ok(snapshot) = MemorySnapshot::read() else {
            warn!("Unable to sample lqosd memory state from /proc");
            continue;
        };

        state.events_for(&snapshot).log(&snapshot);
    }
}

#[derive(Default)]
struct MemoryWatchdogState {
    warned_about_host_pressure: bool,
    warned_about_process_pressure: bool,
}

impl MemoryWatchdogState {
    fn events_for(&mut self, snapshot: &MemorySnapshot) -> MemoryWatchdogEvents {
        let mut events = MemoryWatchdogEvents::default();

        if snapshot.host_memory_pressure() {
            if !self.warned_about_host_pressure {
                events.host_pressure = true;
                self.warned_about_host_pressure = true;
            }
        } else {
            self.warned_about_host_pressure = false;
        }

        if snapshot.process_memory_critical() {
            if !self.warned_about_process_pressure {
                events.process_pressure = true;
                self.warned_about_process_pressure = true;
            }
        } else {
            self.warned_about_process_pressure = false;
        }

        events
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MemoryWatchdogEvents {
    host_pressure: bool,
    process_pressure: bool,
}

impl MemoryWatchdogEvents {
    fn log(&self, snapshot: &MemorySnapshot) {
        if !self.host_pressure && !self.process_pressure {
            return;
        }

        if self.host_pressure {
            log_host_memory_pressure(snapshot);
        }
        if self.process_pressure {
            log_process_memory_critical(snapshot);
        }

        let diagnostics = MemoryDiagnostics::read();
        if self.process_pressure {
            diagnostics.log(MemoryDiagnosticLevel::Error);
        } else {
            diagnostics.log(MemoryDiagnosticLevel::Warn);
        }
    }
}

fn log_host_memory_pressure(snapshot: &MemorySnapshot) {
    warn!(
        "lqosd host memory pressure: mem_available={} mem_total={} threshold={} process_rss={} process_swap={} process_total={} threads={}",
        format_bytes(snapshot.mem_available_bytes),
        format_bytes(snapshot.mem_total_bytes),
        format_bytes(snapshot.host_memory_pressure_threshold_bytes()),
        format_bytes(snapshot.process_rss_bytes),
        format_bytes(snapshot.process_swap_bytes),
        format_bytes(snapshot.process_total_bytes()),
        snapshot.thread_count,
    );
}

fn log_process_memory_critical(snapshot: &MemorySnapshot) {
    error!(
        "lqosd process memory critical: process_total={} mem_total={} threshold={} mem_available={} process_rss={} process_swap={} threads={}",
        format_bytes(snapshot.process_total_bytes()),
        format_bytes(snapshot.mem_total_bytes),
        format_bytes(snapshot.process_memory_critical_threshold_bytes()),
        format_bytes(snapshot.mem_available_bytes),
        format_bytes(snapshot.process_rss_bytes),
        format_bytes(snapshot.process_swap_bytes),
        snapshot.thread_count,
    );
}

enum MemoryDiagnosticLevel {
    Warn,
    Error,
}

struct MemoryDiagnostics {
    flows_tracked: u64,
    bus_requests: u64,
    time_to_poll_hosts_us: u64,
    high_watermark_down: u64,
    high_watermark_up: u64,
}

impl MemoryDiagnostics {
    fn read() -> Self {
        Self {
            flows_tracked: live_active_flow_count(),
            bus_requests: BUS_REQUESTS.load(Ordering::Relaxed),
            time_to_poll_hosts_us: TIME_TO_POLL_HOSTS.load(Ordering::Relaxed),
            high_watermark_down: HIGH_WATERMARK.get_down(),
            high_watermark_up: HIGH_WATERMARK.get_up(),
        }
    }

    fn log(&self, level: MemoryDiagnosticLevel) {
        let flows_tracked = self.flows_tracked;
        let bus_requests = self.bus_requests;
        let time_to_poll_hosts_us = self.time_to_poll_hosts_us;
        let high_watermark_down = self.high_watermark_down;
        let high_watermark_up = self.high_watermark_up;

        match level {
            MemoryDiagnosticLevel::Warn => warn!(
                "lqosd memory diagnostics: flows_tracked={flows_tracked} bus_requests={bus_requests} time_to_poll_hosts_us={time_to_poll_hosts_us} high_watermark_down={high_watermark_down} high_watermark_up={high_watermark_up}",
            ),
            MemoryDiagnosticLevel::Error => error!(
                "lqosd memory diagnostics: flows_tracked={flows_tracked} bus_requests={bus_requests} time_to_poll_hosts_us={time_to_poll_hosts_us} high_watermark_down={high_watermark_down} high_watermark_up={high_watermark_up}",
            ),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct MemorySnapshot {
    mem_total_bytes: u64,
    mem_available_bytes: u64,
    process_rss_bytes: u64,
    process_swap_bytes: u64,
    thread_count: u64,
}

impl MemorySnapshot {
    fn read() -> anyhow::Result<Self> {
        let meminfo = parse_proc_key_values(&std::fs::read_to_string("/proc/meminfo")?);
        let status = parse_proc_key_values(&std::fs::read_to_string("/proc/self/status")?);

        Ok(Self {
            mem_total_bytes: read_required_kb(&meminfo, "MemTotal")?,
            mem_available_bytes: read_required_kb(&meminfo, "MemAvailable")?,
            process_rss_bytes: read_required_kb(&status, "VmRSS")?,
            process_swap_bytes: read_optional_kb(&status, "VmSwap"),
            thread_count: read_optional_raw(&status, "Threads"),
        })
    }

    fn process_total_bytes(&self) -> u64 {
        self.process_rss_bytes
            .saturating_add(self.process_swap_bytes)
    }

    fn host_memory_pressure_threshold_bytes(&self) -> u64 {
        percent_of(self.mem_total_bytes, HOST_PRESSURE_AVAILABLE_PERCENT)
    }

    fn process_memory_critical_threshold_bytes(&self) -> u64 {
        percent_of(self.mem_total_bytes, PROCESS_CRITICAL_TOTAL_RAM_PERCENT)
    }

    fn host_memory_pressure(&self) -> bool {
        self.mem_total_bytes > 0
            && self.mem_available_bytes < self.host_memory_pressure_threshold_bytes()
    }

    fn process_memory_critical(&self) -> bool {
        self.mem_total_bytes > 0
            && self.process_total_bytes() >= self.process_memory_critical_threshold_bytes()
    }
}

fn parse_proc_key_values(input: &str) -> HashMap<String, u64> {
    input
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let value = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key.to_string(), value))
        })
        .collect()
}

fn read_required_kb(values: &HashMap<String, u64>, key: &str) -> anyhow::Result<u64> {
    values
        .get(key)
        .copied()
        .map(kib)
        .ok_or_else(|| anyhow::anyhow!("{key} missing from /proc memory data"))
}

fn read_optional_kb(values: &HashMap<String, u64>, key: &str) -> u64 {
    values.get(key).copied().map(kib).unwrap_or(0)
}

fn read_optional_raw(values: &HashMap<String, u64>, key: &str) -> u64 {
    values.get(key).copied().unwrap_or(0)
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

const fn kib(value: u64) -> u64 {
    value * 1024
}

const fn mib(value: u64) -> u64 {
    value * 1024 * 1024
}

#[cfg(test)]
const fn gib(value: u64) -> u64 {
    value * 1024 * 1024 * 1024
}

const fn percent_of(value: u64, percent: u64) -> u64 {
    value.saturating_mul(percent) / 100
}

fn format_bytes(bytes: u64) -> String {
    let mib_value = bytes as f64 / mib(1) as f64;
    format!("{mib_value:.1} MiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_parser_reads_kb_and_raw_values() {
        let parsed = parse_proc_key_values(
            "MemTotal:       15458992 kB\nMemAvailable:   12843176 kB\nThreads:\t44\n",
        );

        assert_eq!(
            read_required_kb(&parsed, "MemTotal").expect("MemTotal should parse"),
            15_458_992 * 1024
        );
        assert_eq!(read_optional_raw(&parsed, "Threads"), 44);
        assert_eq!(read_optional_kb(&parsed, "VmSwap"), 0);
    }

    #[test]
    fn watchdog_reports_process_memory_at_ninety_percent_of_installed_ram() {
        let snapshot = MemorySnapshot {
            mem_total_bytes: gib(10),
            mem_available_bytes: gib(2),
            process_rss_bytes: gib(9),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert!(snapshot.process_memory_critical());
    }

    #[test]
    fn watchdog_ignores_process_memory_below_ninety_percent_of_installed_ram() {
        let snapshot = MemorySnapshot {
            mem_total_bytes: gib(10),
            mem_available_bytes: gib(2),
            process_rss_bytes: gib(9) - 1,
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert!(!snapshot.process_memory_critical());
    }

    #[test]
    fn watchdog_reports_low_available_host_memory() {
        let snapshot = MemorySnapshot {
            mem_total_bytes: gib(16),
            mem_available_bytes: mib(1_500),
            process_rss_bytes: mib(256),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert!(snapshot.host_memory_pressure());
    }

    #[test]
    fn watchdog_ignores_four_gib_process_on_large_host() {
        let snapshot = MemorySnapshot {
            mem_total_bytes: gib(64),
            mem_available_bytes: gib(48),
            process_rss_bytes: gib(4),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert!(!snapshot.process_memory_critical());
        assert!(!snapshot.host_memory_pressure());
    }

    #[test]
    fn watchdog_state_logs_host_pressure_once_until_recovery() {
        let mut state = MemoryWatchdogState::default();
        let pressure = MemorySnapshot {
            mem_total_bytes: gib(16),
            mem_available_bytes: mib(1_500),
            process_rss_bytes: mib(256),
            process_swap_bytes: 0,
            thread_count: 44,
        };
        let recovered = MemorySnapshot {
            mem_total_bytes: gib(16),
            mem_available_bytes: gib(4),
            process_rss_bytes: mib(256),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert_eq!(
            state.events_for(&pressure),
            MemoryWatchdogEvents {
                host_pressure: true,
                process_pressure: false,
            }
        );
        assert_eq!(state.events_for(&pressure), MemoryWatchdogEvents::default());
        assert_eq!(
            state.events_for(&recovered),
            MemoryWatchdogEvents::default()
        );
        assert_eq!(
            state.events_for(&pressure),
            MemoryWatchdogEvents {
                host_pressure: true,
                process_pressure: false,
            }
        );
    }

    #[test]
    fn watchdog_state_logs_process_pressure_once_until_recovery() {
        let mut state = MemoryWatchdogState::default();
        let pressure = MemorySnapshot {
            mem_total_bytes: gib(16),
            mem_available_bytes: gib(12),
            process_rss_bytes: gib(15),
            process_swap_bytes: mib(400),
            thread_count: 44,
        };
        let recovered = MemorySnapshot {
            mem_total_bytes: gib(16),
            mem_available_bytes: gib(12),
            process_rss_bytes: gib(2),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert_eq!(
            state.events_for(&pressure),
            MemoryWatchdogEvents {
                host_pressure: false,
                process_pressure: true,
            }
        );
        assert_eq!(state.events_for(&pressure), MemoryWatchdogEvents::default());
        assert_eq!(
            state.events_for(&recovered),
            MemoryWatchdogEvents::default()
        );
        assert_eq!(
            state.events_for(&pressure),
            MemoryWatchdogEvents {
                host_pressure: false,
                process_pressure: true,
            }
        );
    }

    #[test]
    fn watchdog_state_reports_both_memory_events_in_one_sample() {
        let mut state = MemoryWatchdogState::default();
        let pressure = MemorySnapshot {
            mem_total_bytes: gib(10),
            mem_available_bytes: mib(512),
            process_rss_bytes: gib(9),
            process_swap_bytes: 0,
            thread_count: 44,
        };

        assert_eq!(
            state.events_for(&pressure),
            MemoryWatchdogEvents {
                host_pressure: true,
                process_pressure: true,
            }
        );
        assert_eq!(state.events_for(&pressure), MemoryWatchdogEvents::default());
    }
}
