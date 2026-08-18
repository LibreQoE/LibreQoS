use crate::node_manager::ws::messages::{PingState, WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::try_send_private_payload;
use lqos_probe::{ProbeClass, ProbeClient};
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const UI_MONITOR_PROBE_MAX_AGE: Duration = Duration::from_millis(250);

pub(super) async fn ping_monitor(
    ip_addresses: Vec<(String, String)>,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
    probe_client: ProbeClient,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;

        let observations = match probe_client
            .probe_reachability_batch(
                ip_addresses.iter().map(|(ip, _)| ip.clone()),
                ProbeClass::UiMonitor,
                Duration::from_secs(1),
                UI_MONITOR_PROBE_MAX_AGE,
            )
            .await
        {
            Ok(observations) => observations,
            Err(error) => {
                debug!("Ping monitor probe provider stopped: {error}");
                break;
            }
        };

        'observations: for ((ip, label), observation) in ip_addresses.iter().zip(observations) {
            if observation.reachable {
                let ping_time = observation
                    .rtt_ms
                    .map(|rtt_ms| Duration::from_secs_f64(rtt_ms / 1000.0))
                    .unwrap_or_else(|| Duration::from_secs(0));
                if !send_alive(&tx, ip.clone(), ping_time, label.clone()) {
                    break 'observations;
                }
            } else {
                if let Some(error) = observation.error.as_deref() {
                    debug!(
                        "Ping monitor target {} did not respond: {}",
                        observation.normalized_target, error
                    );
                }
                if !send_timeout(&tx, ip.clone()) {
                    break 'observations;
                }
            }
        }

        let channel_test = WsResponse::PingMonitor {
            ip: "test".to_string(),
            result: PingState::ChannelTest,
        };
        if let Ok(payload) = encode_ws_message(&channel_test) {
            if !try_send_private_payload(&tx, payload, "PingMonitor") {
                debug!("Channel is gone");
                break;
            }
        } else {
            break;
        }
    }
}

fn send_timeout(tx: &tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>, ip: String) -> bool {
    let result = WsResponse::PingMonitor {
        ip,
        result: PingState::NoResponse,
    };
    encode_ws_message(&result)
        .map(|payload| try_send_private_payload(tx, payload, "PingMonitor"))
        .unwrap_or(false)
}

fn send_alive(
    tx: &tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
    ip: String,
    ping_time: Duration,
    label: String,
) -> bool {
    let result = WsResponse::PingMonitor {
        ip,
        result: PingState::Ping {
            time_nanos: ping_time.as_nanos() as u64,
            label,
        },
    };
    encode_ws_message(&result)
        .map(|payload| try_send_private_payload(tx, payload, "PingMonitor"))
        .unwrap_or(false)
}
