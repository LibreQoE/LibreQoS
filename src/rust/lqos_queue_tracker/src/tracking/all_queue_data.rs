use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use lqos_utils::units::{AtomicDownUp, DownUpOrder};
use crate::tracking::TrackedQueue;

pub static ALL_QUEUE_SUMMARY: Lazy<AllQueueData> = Lazy::new(|| AllQueueData::new());
pub static TOTAL_QUEUE_STATS: TotalQueueStats = TotalQueueStats::new();

pub struct TotalQueueStats {
    pub drops: AtomicDownUp,
    pub marks: AtomicDownUp,
}

impl TotalQueueStats {
    pub const fn new() -> Self {
        Self {
            drops: AtomicDownUp::zeroed(),
            marks: AtomicDownUp::zeroed(),
        }
    }
}

#[derive(Debug)]
pub struct QueueData {
    pub drops: DownUpOrder<u64>,
    pub marks: DownUpOrder<u64>,
    pub prev_drops: Option<DownUpOrder<u64>>,
    pub prev_marks: Option<DownUpOrder<u64>>,
}

fn zero_total_queue_stats() {
    TOTAL_QUEUE_STATS.drops.set_to_zero();
    TOTAL_QUEUE_STATS.marks.set_to_zero();
}

#[derive(Debug)]
pub struct AllQueueData {
    data: Mutex<HashMap<String, QueueData>>,
}

impl AllQueueData {
    pub fn new() -> Self {
        Self { data: Mutex::new(HashMap::new()) }
    }

    pub fn clear(&self) {
        let mut lock = self.data.lock().unwrap();
        lock.clear();
    }

    pub fn ingest_batch(&self, download: Vec<TrackedQueue>, upload: Vec<TrackedQueue>) {
        let mut lock = self.data.lock().unwrap();

        // Roll through moving current to previous
        for (_, q) in lock.iter_mut() {
            q.prev_drops = Some(q.drops);
            q.prev_marks = Some(q.marks);
            q.drops = DownUpOrder::zeroed();
            q.marks = DownUpOrder::zeroed();
        }

        // Make download markings
        for dl in download.into_iter() {
            if let Some(q) = lock.get_mut(&dl.circuit_id) {
                // We need to update it
                q.drops.down = dl.drops;
                q.marks.down = dl.marks;
            } else {
                // We need to add it
                let mut new_record = QueueData {
                    drops: Default::default(),
                    marks: Default::default(),
                    prev_drops: None,
                    prev_marks: None,
                };
                new_record.drops.down = dl.drops;
                new_record.marks.down = dl.marks;
                lock.insert(dl.circuit_id.clone(), new_record);
            }
        }

        // Make upload markings
        for ul in upload.into_iter() {
            if let Some(q) = lock.get_mut(&ul.circuit_id) {
                // We need to update it
                q.drops.up = ul.drops;
                q.marks.up = ul.marks;
            } else {
                // We need to add it
                let mut new_record = QueueData {
                    drops: Default::default(),
                    marks: Default::default(),
                    prev_drops: Default::default(),
                    prev_marks: Default::default(),
                };
                new_record.drops.up = ul.drops;
                new_record.marks.up = ul.marks;
                lock.insert(ul.circuit_id.clone(), new_record);
            }
        }
    }

    pub fn iterate_queues(&self, f: impl Fn(&str, &DownUpOrder<u64>, &DownUpOrder<u64>)) {
        let lock = self.data.lock().unwrap();
        for (circuit_id, q) in lock.iter() {
            if let Some(prev_drops) = q.prev_drops {
                if let Some(prev_marks) = q.prev_marks {
                    if q.drops > prev_drops || q.marks > prev_marks {
                        let drops = q.drops.checked_sub_or_zero(prev_drops);
                        let marks = q.marks.checked_sub_or_zero(prev_marks);
                        f(circuit_id, &drops, &marks);
                    }
                }
            }
        }
    }

    pub fn calculate_total_queue_stats(&self) {
        zero_total_queue_stats();
        let lock = self.data.lock().unwrap();

        let mut drops = DownUpOrder::zeroed();
        let mut marks = DownUpOrder::zeroed();

        lock
            .iter()
            .filter(|(_, q)| q.prev_drops.is_some() && q.prev_marks.is_some())
            .for_each(|(_, q)| {
                drops += q.drops.checked_sub_or_zero(q.prev_drops.unwrap());
                marks += q.marks.checked_sub_or_zero(q.prev_marks.unwrap());
            });

        TOTAL_QUEUE_STATS.drops.set_down(drops.down);
        TOTAL_QUEUE_STATS.drops.set_up(drops.up);
        TOTAL_QUEUE_STATS.marks.set_down(marks.down);
        TOTAL_QUEUE_STATS.marks.set_up(marks.up);
    }
}