use crate::config::StormguardConfig;
use lqos_config::StormguardStrategy;
use lqos_probe::{ProbeClass, ProbeClient};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const STORMGUARD_PROBE_MAX_AGE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug)]
pub struct TimedRtt {
    pub rtt_ms: f64,
    pub at: Instant,
}

#[derive(Clone, Debug, PartialEq)]
struct PingSettings {
    target: String,
    interval: Duration,
    timeout: Duration,
}

pub struct ActivePingManager {
    probe_client: ProbeClient,
    settings: Option<PingSettings>,
    rx: Option<watch::Receiver<Option<TimedRtt>>>,
    handle: Option<tokio::task::JoinHandle<()>>,
    last_seen_at: Option<Instant>,
}

impl ActivePingManager {
    pub fn new(probe_client: ProbeClient) -> Self {
        Self {
            probe_client,
            settings: None,
            rx: None,
            handle: None,
            last_seen_at: None,
        }
    }

    pub fn reconfigure(&mut self, cfg: Option<&StormguardConfig>) {
        let desired = cfg
            .filter(|c| c.strategy == StormguardStrategy::DelayProbeActive)
            .map(|c| PingSettings {
                target: c.active_ping_target.trim().to_string(),
                interval: Duration::from_secs_f32(c.active_ping_interval_seconds.max(1.0)),
                timeout: Duration::from_secs_f32(c.active_ping_timeout_seconds.max(0.1)),
            });

        if desired == self.settings {
            return;
        }

        self.stop();

        let Some(settings) = desired.clone() else {
            return;
        };

        let (tx, rx) = watch::channel(None);
        self.settings = desired;
        self.rx = Some(rx);
        self.handle = Some(tokio::spawn(ping_loop(
            settings,
            tx,
            self.probe_client.clone(),
        )));
    }

    pub fn latest(&mut self) -> (Option<TimedRtt>, bool) {
        let Some(rx) = &self.rx else {
            self.last_seen_at = None;
            return (None, false);
        };

        let sample = *rx.borrow();
        let updated = sample
            .as_ref()
            .map(|s| s.at)
            .is_some_and(|at| Some(at) != self.last_seen_at);
        if updated {
            self.last_seen_at = sample.as_ref().map(|s| s.at);
        }
        (sample, updated)
    }

    fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        self.settings = None;
        self.rx = None;
        self.last_seen_at = None;
    }
}

async fn ping_loop(
    settings: PingSettings,
    tx: watch::Sender<Option<TimedRtt>>,
    probe_client: ProbeClient,
) {
    let mut ticker = tokio::time::interval(settings.interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match probe_client
            .probe_round_trip_time(
                settings.target.clone(),
                ProbeClass::Stormguard,
                settings.timeout,
                STORMGUARD_PROBE_MAX_AGE,
            )
            .await
        {
            Ok(observation) if observation.reachable => {
                if let Some(rtt_ms) = observation.rtt_ms {
                    let _ = tx.send(Some(TimedRtt {
                        rtt_ms,
                        at: Instant::now(),
                    }));
                }
            }
            Ok(observation) => {
                debug!(
                    "StormGuard active ping to {} failed: {}",
                    observation.normalized_target,
                    observation
                        .error
                        .unwrap_or_else(|| "no response".to_string())
                );
            }
            Err(error) => debug!("StormGuard active ping provider error: {error}"),
        }
    }
}
