use crate::config::StormguardConfig;
use lqos_config::StormguardStrategy;
use rand::random;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence};
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use tracing::{debug, warn};

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
    settings: Option<PingSettings>,
    rx: Option<watch::Receiver<Option<TimedRtt>>>,
    handle: Option<tokio::task::JoinHandle<()>>,
    last_seen_at: Option<Instant>,
}

impl ActivePingManager {
    pub fn new() -> Self {
        Self {
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
        self.handle = Some(tokio::spawn(ping_loop(settings, tx)));
    }

    pub fn latest(&mut self) -> (Option<TimedRtt>, bool) {
        let Some(rx) = &self.rx else {
            self.last_seen_at = None;
            return (None, false);
        };

        let sample = rx.borrow().clone();
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

async fn ping_loop(settings: PingSettings, tx: watch::Sender<Option<TimedRtt>>) {
    let mut ticker = tokio::time::interval(settings.interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let Some(target) = resolve_target(&settings.target).await else {
            warn!(
                "StormGuard active ping target '{}' could not be resolved",
                settings.target
            );
            continue;
        };

        match ping_once(target, settings.timeout).await {
            Some(rtt_ms) => {
                let _ = tx.send(Some(TimedRtt {
                    rtt_ms,
                    at: Instant::now(),
                }));
            }
            None => debug!("StormGuard active ping to {} failed", target),
        }
    }
}

async fn resolve_target(target: &str) -> Option<IpAddr> {
    if let Ok(ip) = target.parse::<IpAddr>() {
        return Some(ip);
    }

    let mut addrs = tokio::net::lookup_host((target, 80)).await.ok()?;
    addrs.next().map(|a| a.ip())
}

async fn ping_once(ip: IpAddr, timeout: Duration) -> Option<f64> {
    let client = match ip {
        IpAddr::V4(_) => Client::new(&Config::default()).ok()?,
        IpAddr::V6(_) => Client::new(&Config::builder().kind(ICMP::V6).build()).ok()?,
    };

    let payload = [0; 56];
    let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
    pinger.timeout(timeout);
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((IcmpPacket::V4(..), dur)) | Ok((IcmpPacket::V6(..), dur)) => {
            Some(dur.as_secs_f64() * 1000.0)
        }
        _ => None,
    }
}

