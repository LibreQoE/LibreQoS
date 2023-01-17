use std::time::{Duration, Instant};
use rayon::prelude::{IntoParallelRefMutIterator, ParallelIterator};
use tokio::time;
use lqos_config::LibreQoSConfig;
use anyhow::Result;
use tokio::task;
use crate::{circuit_to_queue::CIRCUIT_TO_QUEUE, queue_store::QueueStore, interval::QUEUE_MONITOR_INTERVAL, tracking::reader::read_named_queue_from_interface};
use log::info;
mod reader;
mod watched_queues;
pub use watched_queues::{add_watched_queue, still_watching};
use watched_queues::WATCHED_QUEUES;

use self::watched_queues::expire_watched_queues;

async fn track_queues() -> Result<()> {
    let mut watching = WATCHED_QUEUES.write();
    if watching.is_empty() {
        return Ok(()) // There's nothing to do - bail out fast
    }
    let config = LibreQoSConfig::load()?;
    watching.par_iter_mut().for_each(|q| {
            let (circuit_id, download_class, upload_class) = q.get();

            let (download, upload) = if config.on_a_stick_mode {
                (
                    read_named_queue_from_interface(&config.internet_interface, download_class),
                    read_named_queue_from_interface(&config.internet_interface, upload_class)
                )
            } else {
                (
                    read_named_queue_from_interface(&config.isp_interface, download_class),
                    read_named_queue_from_interface(&config.internet_interface, download_class)
                )
            };

            if let Ok(download) = download {
                if let Ok(upload) = upload {
                    let mut mapping = CIRCUIT_TO_QUEUE.write();
                    if let Some(circuit) = mapping.get_mut(circuit_id) {
                        circuit.update(&download[0], &upload[0]);
                    } else {
                        // It's new: insert it
                        mapping.insert(
                            circuit_id.to_string(),
                            QueueStore::new(download[0].clone(), upload[0].clone()),
                        );
                    }
                }
        }
        });

    std::mem::drop(watching); // Release the lock
    expire_watched_queues();
    Ok(())
}

pub async fn spawn_queue_monitor() {
    let _ = task::spawn(async {
        QUEUE_MONITOR_INTERVAL.store(lqos_config::EtcLqos::load().unwrap().queue_check_period_ms, std::sync::atomic::Ordering::Relaxed);
        loop {
            let queue_check_period_ms = QUEUE_MONITOR_INTERVAL.load(std::sync::atomic::Ordering::Relaxed);
            let mut interval = time::interval(Duration::from_millis(queue_check_period_ms));

            let now = Instant::now();
            let _ = track_queues().await;
            let elapsed = now.elapsed();
            info!("TC Reader tick with mapping consumed {} ms.", elapsed.as_millis());
            if elapsed.as_millis() < queue_check_period_ms as u128 {
                let duration = Duration::from_millis(queue_check_period_ms) - elapsed;
                tokio::time::sleep(duration).await;
            } else {
                interval.tick().await;
            }
        }
    });
}