use self::queue_reader::{make_queue_diff, QueueDiff, QueueType};
use crate::libreqos_tracker::QUEUE_STRUCTURE;
use lqos_bus::BusResponse;
use lqos_config::LibreQoSConfig;
use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant}, sync::atomic::AtomicU64,
};
use tokio::{join, task, time};
mod queue_reader;
use lazy_static::*;
use parking_lot::RwLock;
use anyhow::Result;

const NUM_QUEUE_HISTORY: usize = 600;

#[derive(Debug, Serialize)]
pub struct QueueStore {
    history: Vec<(QueueDiff, QueueDiff)>,
    history_head: usize,
    prev_download: Option<QueueType>,
    prev_upload: Option<QueueType>,
    current_download: QueueType,
    current_upload: QueueType,
}

impl QueueStore {
    fn new(download: QueueType, upload: QueueType) -> Self {
        Self {
            history: vec![(QueueDiff::None, QueueDiff::None); NUM_QUEUE_HISTORY],
            history_head: 0,
            prev_upload: None,
            prev_download: None,
            current_download: download,
            current_upload: upload,
        }
    }

    fn update(&mut self, download: &QueueType, upload: &QueueType) {
        self.prev_upload = Some(self.current_upload.clone());
        self.prev_download = Some(self.current_download.clone());
        self.current_download = download.clone();
        self.current_upload = upload.clone();
        let new_diff_up = make_queue_diff(self.prev_upload.as_ref().unwrap(), &self.current_upload);
        let new_diff_dn =
            make_queue_diff(self.prev_download.as_ref().unwrap(), &self.current_download);
        if new_diff_dn.is_ok() && new_diff_up.is_ok() {
            self.history[self.history_head] = (new_diff_dn.unwrap(), new_diff_up.unwrap());
            self.history_head += 1;
            if self.history_head >= NUM_QUEUE_HISTORY {
                self.history_head = 0;
            }
        }
    }
}

lazy_static! {
    pub(crate) static ref CIRCUIT_TO_QUEUE: RwLock<HashMap<String, QueueStore>> =
        RwLock::new(HashMap::new());
}

async fn track_queues() -> Result<()> {
    let config = LibreQoSConfig::load()?;
    let queues = if config.on_a_stick_mode {
        let queues = queue_reader::read_tc_queues(&config.internet_interface)
            .await?;
        vec![queues]
    } else {
        let (isp, internet) = join! {
            queue_reader::read_tc_queues(&config.isp_interface),
            queue_reader::read_tc_queues(&config.internet_interface),
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

lazy_static! {
    pub(crate) static ref QUEUE_MONITOR_INTERVAL: AtomicU64 = AtomicU64::new(1000);
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

pub fn get_raw_circuit_data(circuit_id: &str) -> BusResponse {
    let reader = CIRCUIT_TO_QUEUE.read();
    if let Some(circuit) = reader.get(circuit_id) {
        if let Ok(json) = serde_json::to_string(circuit) {
            BusResponse::RawQueueData(json)
        } else {
            BusResponse::RawQueueData(String::new())
        }
    } else {
        BusResponse::RawQueueData(String::new())
    }
}
