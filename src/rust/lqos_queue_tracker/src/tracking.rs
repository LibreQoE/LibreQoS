use std::time::{Duration, Instant};
use tokio::time;
use lqos_config::LibreQoSConfig;
use anyhow::Result;
use tokio::{join, task};
use crate::{queue_types::{read_tc_queues, QueueType}, circuit_to_queue::CIRCUIT_TO_QUEUE, queue_structure::QUEUE_STRUCTURE, queue_store::QueueStore, interval::QUEUE_MONITOR_INTERVAL};

async fn track_queues() -> Result<()> {
    let config = LibreQoSConfig::load()?;
    let queues = if config.on_a_stick_mode {
        let queues = read_tc_queues(&config.internet_interface)
            .await?;
        vec![queues]
    } else {
        let (isp, internet) = join! {
            read_tc_queues(&config.isp_interface),
            read_tc_queues(&config.internet_interface),
        };
        vec![isp?, internet?]
    };

    // Time to associate queues with circuits
    let mut mapping = CIRCUIT_TO_QUEUE.write();
    let structure_lock = QUEUE_STRUCTURE.read();

    // Do a quick check that we have a queue association
    if let Ok(structure) = &*structure_lock {
        for circuit in structure.iter().filter(|c| c.circuit_id.is_some()) {
            if config.on_a_stick_mode {
                let download = queues[0].iter().find(|q| match q {
                    QueueType::Cake(cake) => {
                        let (maj, min) = cake.parent.get_major_minor();
                        let (cmaj, cmin) = circuit.class_id.get_major_minor();
                        maj == cmaj && min == cmin
                    }
                    QueueType::FqCodel(fq) => fq.parent.as_u32() == circuit.class_id.as_u32(),
                    _ => false,
                });
                let upload = queues[0].iter().find(|q| match q {
                    QueueType::Cake(cake) => {
                        let (maj, min) = cake.parent.get_major_minor();
                        let (cmaj, cmin) = circuit.up_class_id.get_major_minor();
                        maj == cmaj && min == cmin
                    }
                    QueueType::FqCodel(fq) => fq.parent.as_u32() == circuit.up_class_id.as_u32(),
                    _ => false,
                });
                if let Some(download) = download {
                    if let Some(upload) = upload {
                        if let Some(circuit_id) = &circuit.circuit_id {
                            if let Some(circuit) = mapping.get_mut(circuit_id) {
                                circuit.update(download, upload);
                            } else {
                                // It's new: insert it
                                mapping.insert(
                                    circuit_id.clone(),
                                    QueueStore::new(download.clone(), upload.clone()),
                                );
                            }
                        }
                    }
                }
            } else {
                let download = queues[0].iter().find(|q| match q {
                    QueueType::Cake(cake) => {
                        let (maj, min) = cake.parent.get_major_minor();
                        let (cmaj, cmin) = circuit.class_id.get_major_minor();
                        maj == cmaj && min == cmin
                    }
                    QueueType::FqCodel(fq) => fq.parent.as_u32() == circuit.class_id.as_u32(),
                    _ => false,
                });
                let upload = queues[1].iter().find(|q| match q {
                    QueueType::Cake(cake) => {
                        let (maj, min) = cake.parent.get_major_minor();
                        let (cmaj, cmin) = circuit.class_id.get_major_minor();
                        maj == cmaj && min == cmin
                    }
                    QueueType::FqCodel(fq) => fq.parent.as_u32() == circuit.class_id.as_u32(),
                    _ => false,
                });
                if let Some(download) = download {
                    if let Some(upload) = upload {
                        if let Some(circuit_id) = &circuit.circuit_id {
                            if let Some(circuit) = mapping.get_mut(circuit_id) {
                                circuit.update(download, upload);
                            } else {
                                // It's new: insert it
                                mapping.insert(
                                    circuit_id.clone(),
                                    QueueStore::new(download.clone(), upload.clone()),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

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
            //println!("TC Reader tick with mapping consumed {} ms.", elapsed.as_millis());
            if elapsed.as_millis() < queue_check_period_ms as u128 {
                let duration = Duration::from_millis(queue_check_period_ms) - elapsed;
                //println!("Sleeping for {:.2} seconds", duration.as_secs_f32());
                tokio::time::sleep(duration).await;
            } else {
                interval.tick().await;
            }
        }
    });
}