use lqos_utils::units::{AtomicDownUp, DownUpOrder};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;

/// Counts of queues observed on the system.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueCounts {
    /// Number of CAKE queues.
    pub cake: usize,
    /// Number of fq_codel queues.
    pub fq_codel: usize,
    /// Number of HTB queues.
    pub htb: usize,
}

/// Holds all of the CAKE queue summaries being tracked by the system.
pub static ALL_QUEUE_SUMMARY: Lazy<AllQueueData> = Lazy::new(AllQueueData::new);

/// Tracks the total number of drops and marks across all queues.
pub static TOTAL_QUEUE_STATS: TotalQueueStats = TotalQueueStats::new();

/// Per-circuit aggregate CAKE drops/marks.
pub struct TotalQueueStats {
    /// Total drops across all queues.
    pub drops: AtomicDownUp,
    /// Total marks across all queues.
    pub marks: AtomicDownUp,
}

impl TotalQueueStats {
    /// Builds a zeroed counter pair.
    pub const fn new() -> Self {
        Self {
            drops: AtomicDownUp::zeroed(),
            marks: AtomicDownUp::zeroed(),
        }
    }
}

/// Per-circuit aggregate CAKE drops/marks.
#[derive(Debug)]
pub struct QueueData {
    /// Current drops.
    pub drops: DownUpOrder<u64>,
    /// Current marks.
    pub marks: DownUpOrder<u64>,
    /// Previous drops.
    pub prev_drops: Option<DownUpOrder<u64>>,
    /// Previous marks.
    pub prev_marks: Option<DownUpOrder<u64>>,
}

#[derive(Debug)]
/// The shared aggregate queue store.
pub struct AllQueueData {
    // Map is keyed on circuit hash (which is a hash of circuit_id)
    data: Mutex<HashMap<i64, QueueData>>,
    counts: Mutex<QueueCounts>,
}

impl AllQueueData {
    /// Builds an empty store.
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            counts: Mutex::new(QueueCounts::default()),
        }
    }

    /// Clears the store.
    pub fn clear(&self) {
        let mut lock = self.data.lock();
        lock.clear();
        *self.counts.lock() = QueueCounts::default();
    }

    /// 3a A1: replace the per-circuit store from ONE bridge qdisc read, keeping
    /// each circuit's previous counters so [`AllQueueData::iterate_queues`] keeps
    /// yielding the per-interval DELTA the LTS `circuit_cake_*` contract wants.
    /// Also refreshes the aggregate totals + the observed queue-kind counts.
    /// `current` yields `(circuit_hash, drops, marks)` CUMULATIVE.
    pub fn update(
        &self,
        current: impl IntoIterator<Item = (i64, DownUpOrder<u64>, DownUpOrder<u64>)>,
        counts: QueueCounts,
    ) {
        let mut next: HashMap<i64, QueueData> = HashMap::new();
        {
            let lock = self.data.lock();
            for (circuit_hash, drops, marks) in current {
                let (prev_drops, prev_marks) = match lock.get(&circuit_hash) {
                    Some(previous) => (Some(previous.drops), Some(previous.marks)),
                    None => (None, None),
                };
                next.insert(
                    circuit_hash,
                    QueueData {
                        drops,
                        marks,
                        prev_drops,
                        prev_marks,
                    },
                );
            }
        }
        {
            let mut lock = self.data.lock();
            *lock = next;
        }
        *self.counts.lock() = counts;
        self.calculate_total_queue_stats();
    }

    /// Visits per-circuit delta drops/marks.
    pub fn iterate_queues(&self, mut f: impl FnMut(i64, &DownUpOrder<u64>, &DownUpOrder<u64>)) {
        let lock = self.data.lock();
        for (circuit_id, q) in lock.iter() {
            if let Some(prev_drops) = q.prev_drops
                && let Some(prev_marks) = q.prev_marks
                && (q.drops > prev_drops || q.marks > prev_marks)
            {
                let drops = q.drops.checked_sub_or_zero(prev_drops);
                let marks = q.marks.checked_sub_or_zero(prev_marks);
                f(*circuit_id, &drops, &marks);
            }
        }
    }

    /// Recomputes the aggregate drop/mark counters.
    pub fn calculate_total_queue_stats(&self) {
        TOTAL_QUEUE_STATS.drops.set_to_zero();
        TOTAL_QUEUE_STATS.marks.set_to_zero();
        let lock = self.data.lock();

        let mut drops = DownUpOrder::zeroed();
        let mut marks = DownUpOrder::zeroed();

        lock.iter()
            .filter(|(_, q)| q.prev_drops.is_some() && q.prev_marks.is_some())
            .for_each(|(_, q)| {
                drops += q
                    .drops
                    .checked_sub_or_zero(q.prev_drops.unwrap_or_default());
                marks += q
                    .marks
                    .checked_sub_or_zero(q.prev_marks.unwrap_or_default());
            });

        TOTAL_QUEUE_STATS.drops.set_down(drops.down);
        TOTAL_QUEUE_STATS.drops.set_up(drops.up);
        TOTAL_QUEUE_STATS.marks.set_down(marks.down);
        TOTAL_QUEUE_STATS.marks.set_up(marks.up);
    }

    /// Return the most recently observed queue counts.
    pub fn queue_counts(&self) -> QueueCounts {
        *self.counts.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §lqos-74t.8 A1: the bridge feed keeps each circuit's previous counters, so
    /// `iterate_queues` yields the per-interval DELTA the LTS `circuit_cake_*`
    /// contract wants; the first update only seeds, and counts refresh.
    #[test]
    fn update_keeps_prev_and_yields_the_interval_delta() {
        let store = AllQueueData::new();
        store.update(
            [(1_i64, DownUpOrder::new(10, 5), DownUpOrder::new(2, 1))],
            QueueCounts {
                cake: 2,
                htb: 3,
                fq_codel: 1,
            },
        );
        let mut first = Vec::new();
        store.iterate_queues(|hash, drops, marks| first.push((hash, *drops, *marks)));
        assert!(first.is_empty(), "the first update only seeds (no previous)");
        assert_eq!(store.queue_counts().cake, 2);
        assert_eq!(store.queue_counts().htb, 3);
        store.update(
            [(1_i64, DownUpOrder::new(14, 5), DownUpOrder::new(5, 1))],
            QueueCounts::default(),
        );
        let mut second = Vec::new();
        store.iterate_queues(|hash, drops, marks| second.push((hash, *drops, *marks)));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].1, DownUpOrder::new(4, 0), "drops delta");
        assert_eq!(second[0].2, DownUpOrder::new(3, 0), "marks delta");
        assert_eq!(store.queue_counts().cake, 0);
    }
}
