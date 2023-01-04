use std::{time::{Duration, Instant}, collections::HashMap};
use lqos_bus::BusResponse;
use lqos_config::LibreQoSConfig;
use tokio::{task, time};
use crate::libreqos_tracker::QUEUE_STRUCTURE;
use self::queue_reader::QueueType;
mod queue_reader;
use lazy_static::*;
use parking_lot::RwLock;

lazy_static! {
    pub(crate) static ref CIRCUIT_TO_QUEUE : RwLock<HashMap<String, (QueueType, QueueType)>> = RwLock::new(HashMap::new());
}

fn track_queues() {
    let config = LibreQoSConfig::load().unwrap();
    let queues = if config.on_a_stick_mode {
        let queues = queue_reader::read_tc_queues(&config.internet_interface).unwrap();
        vec![queues]
    } else {
        vec![
            queue_reader::read_tc_queues(&config.isp_interface).unwrap(),
            queue_reader::read_tc_queues(&config.internet_interface).unwrap(),
        ]
    };

    // Time to associate queues with circuits
    let mut mapping: HashMap<String, (QueueType, QueueType)> = HashMap::new();
    let structure_lock = QUEUE_STRUCTURE.read();
    // Do a quick check that we have a queue association
    if let Ok(structure) = &*structure_lock {
        for circuit in structure.iter().filter(|c| c.circuit_id.is_some()) {
            if config.on_a_stick_mode {
                let download = queues[0].iter().find(|q| {
                    match q {
                        QueueType::Cake(cake) => {
                            let (maj,min) = cake.parent.get_major_minor();
                            let (cmaj,cmin) = circuit.class_id.get_major_minor();
                            maj==cmaj && min == cmin
                        }
                        QueueType::FqCodel(fq) => {
                            fq.parent.as_u32() == circuit.class_id.as_u32()
                        }
                        _ => false,
                    }
                });
                let upload = queues[0].iter().find(|q| {
                    match q {
                        QueueType::Cake(cake) => {
                            let (maj,min) = cake.parent.get_major_minor();
                            let (cmaj,cmin) = circuit.up_class_id.get_major_minor();
                            maj==cmaj && min == cmin
                        }
                        QueueType::FqCodel(fq) => {
                            fq.parent.as_u32() == circuit.up_class_id.as_u32()
                        }
                        _ => false,
                    }
                });
                mapping.insert(
                    circuit.circuit_id.as_ref().unwrap().clone(),
                    (download.unwrap().clone(), upload.unwrap().clone())
                );
            } else {
                let download = queues[0].iter().find(|q| {
                    match q {
                        QueueType::Cake(cake) => {
                            let (maj,min) = cake.parent.get_major_minor();
                            let (cmaj,cmin) = circuit.class_id.get_major_minor();
                            maj==cmaj && min == cmin
                        }
                        QueueType::FqCodel(fq) => {
                            fq.parent.as_u32() == circuit.class_id.as_u32()
                        }
                        _ => false,
                    }
                });
                let upload = queues[1].iter().find(|q| {
                    match q {
                        QueueType::Cake(cake) => {
                            let (maj,min) = cake.parent.get_major_minor();
                            let (cmaj,cmin) = circuit.class_id.get_major_minor();
                            maj==cmaj && min == cmin
                        }
                        QueueType::FqCodel(fq) => {
                            fq.parent.as_u32() == circuit.class_id.as_u32()
                        }
                        _ => false,
                    }
                });
                mapping.insert(
                    circuit.circuit_id.as_ref().unwrap().clone(),
                    (download.unwrap().clone(), upload.unwrap().clone())
                );
            }
        }
        *CIRCUIT_TO_QUEUE.write() = mapping;
    }
}

pub async fn spawn_queue_monitor() {
    let _ = task::spawn(async {
        let mut interval = time::interval(Duration::from_secs(10));

        loop {
            let now = Instant::now();
            let _ = task::spawn_blocking(move || {
                track_queues()
            })
            .await;
            let elapsed = now.elapsed();
            //println!("TC Reader tick with mapping consumed {:.4} seconds.", elapsed.as_secs_f32());
            if elapsed.as_secs_f32() < 10.0 {
                let duration = Duration::from_secs(10) - elapsed;
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