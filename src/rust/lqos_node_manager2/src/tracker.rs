use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::atomic::Ordering::Relaxed;
use tokio::sync::mpsc::Receiver;
use crate::ChangeAnnouncement;

pub static FLOW_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static SHAPED_DEVICE_COUNT: AtomicUsize = AtomicUsize::new(0);

pub static TOTAL_BITS_PER_SECOND: (AtomicU64, AtomicU64) = (AtomicU64::new(0), AtomicU64::new(0));
pub static SHAPED_BITS_PER_SECOND: (AtomicU64, AtomicU64) = (AtomicU64::new(0), AtomicU64::new(0));
pub static PACKETS_PER_SECOND: (AtomicU64, AtomicU64) = (AtomicU64::new(0), AtomicU64::new(0));

pub async fn track_changes(mut receiver: Receiver<ChangeAnnouncement>) {
    while let Some(msg) = receiver.recv().await {
        match msg {
            ChangeAnnouncement::FlowCount(count) => FLOW_COUNT.store(count, Relaxed),
            ChangeAnnouncement::ShapedDeviceCount(count) => SHAPED_DEVICE_COUNT.store(count, Relaxed),
            ChangeAnnouncement::ThroughputUpdate { bytes_per_second, shaped_bytes_per_second, packets_per_second } => {
                TOTAL_BITS_PER_SECOND.0.store(bytes_per_second.0 * 8, Relaxed);
                TOTAL_BITS_PER_SECOND.1.store(bytes_per_second.1 * 8, Relaxed);
                SHAPED_BITS_PER_SECOND.0.store(shaped_bytes_per_second.0 * 8, Relaxed);
                SHAPED_BITS_PER_SECOND.1.store(shaped_bytes_per_second.1 * 8, Relaxed);
                PACKETS_PER_SECOND.0.store(packets_per_second.0, Relaxed);
                PACKETS_PER_SECOND.1.store(packets_per_second.1, Relaxed);
            }
        }
    }
}