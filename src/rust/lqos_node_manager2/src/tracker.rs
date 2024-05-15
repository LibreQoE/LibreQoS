use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;
use tokio::sync::mpsc::Receiver;
use crate::ChangeAnnouncement;

pub static FLOW_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static SHAPED_DEVICE_COUNT: AtomicUsize = AtomicUsize::new(0);

pub async fn track_changes(mut receiver: Receiver<ChangeAnnouncement>) {
    while let Some(msg) = receiver.recv().await {
        match msg {
            ChangeAnnouncement::FlowCount(count) => FLOW_COUNT.store(count, Relaxed),
            ChangeAnnouncement::ShapedDeviceCount(count) => SHAPED_DEVICE_COUNT.store(count, Relaxed),
        }
    }
}