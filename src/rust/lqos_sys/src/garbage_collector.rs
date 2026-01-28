use lqos_utils::unix_time::time_since_boot;
use std::time::Duration;
use tracing::{debug, error, info};

/// Starts a periodic garbage collector that will run every hour.
/// This is used to clean up old eBPF map entries to limit memory usage.
pub fn bpf_garbage_collector() {
    //const SLEEP_TIME: u64 = 60 * 60; // 1 Hour
    const SLEEP_TIME: u64 = 5 * 60; // 5 Minutes

    debug!("Starting BPF garbage collector");
    let result = std::thread::Builder::new()
        .name("bpf_garbage_collector".to_string())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(SLEEP_TIME));
                debug!("Running BPF garbage collector");
                throughput_garbage_collect();
            }
        });
    if let Err(e) = result {
        error!("Failed to start BPF garbage collector: {:?}", e);
    }
}

/// Iterates through all throughput entries, building a list of any that
/// haven't been seen for an hour. These are then bulk deleted.
fn throughput_garbage_collect() {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration for garbage collector");
        return;
    };
    let expiry_time_seconds = config.queues.lazy_expire_seconds.unwrap_or(60 * 15); // Default to 15 minutes if not set

    let Ok(now) = time_since_boot() else { return };
    let now = Duration::from(now).as_nanos() as u64;
    let period_nanos = expiry_time_seconds.saturating_mul(1_000_000_000);
    let period_ago = now.saturating_sub(period_nanos);

    let mut expired = Vec::new();
    unsafe {
        crate::bpf_iterator::iterate_throughput(&mut |ip, counters| {
            let last_seen: u64 = counters
                .iter()
                .map(|c| c.last_seen)
                .collect::<Vec<_>>()
                .into_iter()
                .max()
                .unwrap_or(0);
            if last_seen < period_ago {
                expired.push(ip.clone());
            }
        });
    }

    if !expired.is_empty() {
        info!("Garbage collecting {} throughput entries", expired.len());
        if let Err(e) = crate::bpf_iterator::expire_throughput(expired) {
            error!("Failed to garbage collect throughput: {:?}", e);
        }
    }
}
