use tokio::sync::Mutex;
use once_cell::sync::Lazy;
use super::CakeStats;

static CAKE_TRACKER: Lazy<Mutex<CakeTracker>> = Lazy::new(|| Mutex::new(CakeTracker::new()));

pub(crate) async fn update_cake_stats() -> Option<(Vec<CakeStats>, Vec<CakeStats>)> {
    let mut tracker = CAKE_TRACKER.lock().await;
    tracker.update().await
}

pub(crate) struct CakeTracker {
    prev: Option<(Vec<CakeStats>, Vec<CakeStats>)>,
    current: Option<(Vec<CakeStats>, Vec<CakeStats>)>,
}

impl CakeTracker {
    pub(crate) fn new() -> Self {
        Self {
            prev: None,
            current: None,
        }
    }

    pub(crate) async fn update(&mut self) -> Option<(Vec<CakeStats>, Vec<CakeStats>)> {
        if let Ok(cfg) = lqos_config::LibreQoSConfig::load() {
            let outbound = &cfg.internet_interface;
            let inbound = &cfg.isp_interface;
            if cfg.on_a_stick_mode {
                let reader = super::AsyncQueueReader::new(outbound);
                if let Ok((Some(up), Some(down))) = reader.run_on_a_stick().await {
                    return self.read_up_down(up, down);
                }
            } else {
                let out_reader = super::AsyncQueueReader::new(outbound);
                let in_reader = super::AsyncQueueReader::new(inbound);
                let (up, down) = tokio::join!(
                    out_reader.run(),
                    in_reader.run(),
                );
                if let (Ok(Some(up)), Ok(Some(down))) = (up, down) {
                    return self.read_up_down(up, down);
                }
            }
        }
        None
    }

    fn read_up_down(&mut self, up: Vec<CakeStats>, down: Vec<CakeStats>) -> Option<(Vec<CakeStats>, Vec<CakeStats>)> {
        if self.prev.is_none() {
            self.prev = Some((up, down));
            None
        } else {
            // Delta time
            if let Some((down, up)) = &mut self.current {
                down.iter_mut().for_each(|d| {
                    if let Some(prev) = self.prev.as_ref().unwrap().0.iter().find(|p| p.circuit_id == d.circuit_id) {
                        d.drops = d.drops.saturating_sub(prev.drops);
                        d.marks = d.marks.saturating_sub(prev.marks);
                    }
                });
                up.iter_mut().for_each(|d| {
                    if let Some(prev) = self.prev.as_ref().unwrap().1.iter().find(|p| p.circuit_id == d.circuit_id) {
                        d.drops = d.drops.saturating_sub(prev.drops);
                        d.marks = d.marks.saturating_sub(prev.marks);
                    }
                });
            }

            // Advance the previous
            self.prev = self.current.take();

            Some((up, down))
        }
    }
}