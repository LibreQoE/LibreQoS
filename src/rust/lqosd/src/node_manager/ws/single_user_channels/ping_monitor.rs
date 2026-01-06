use crate::node_manager::ws::messages::{PingState, WsResponse, encode_ws_message};
use rand::random;
use std::net::IpAddr;
use std::time::Duration;
use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence};
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info};

pub(super) async fn ping_monitor(
    ip_addresses: Vec<(String, String)>,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    {
        let Ok(cfg) = lqos_config::load_config() else {
            error!("Failed to load configuration for ping monitor");
            return;
        };
        if cfg.disable_icmp_ping.unwrap_or(false) {
            info!("ICMP ping is disabled in the configuration, not starting ping monitor");
            return;
        }
    }
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;

        let Ok(client_v4) = Client::new(&Config::default()) else {
            break;
        };
        let Ok(client_v6) = Client::new(&Config::builder().kind(ICMP::V6).build()) else {
            break;
        };

        let mut tasks = Vec::new();
        for (ip, label) in ip_addresses.iter() {
            match ip.parse() {
                Ok(IpAddr::V4(addr)) => tasks.push(tokio::spawn(ping(
                    client_v4.clone(),
                    IpAddr::V4(addr),
                    tx.clone(),
                    label.clone(),
                ))),
                Ok(IpAddr::V6(addr)) => tasks.push(tokio::spawn(ping(
                    client_v6.clone(),
                    IpAddr::V6(addr),
                    tx.clone(),
                    label.clone(),
                ))),
                Err(e) => error!("{} parse to ipaddr error: {}", ip, e),
            }
        }

        // Use futures to join on all tasks
        for task in tasks {
            let _ = task.await;
        }

        // Notify the channel that we're still here - this is really
        // just a test to see if the channel is still alive
        let channel_test = WsResponse::PingMonitor {
            ip: "test".to_string(),
            result: PingState::ChannelTest,
        };
        if let Ok(payload) = encode_ws_message(&channel_test) {
            if let Err(_) = tx.send(payload).await {
                debug!("Channel is gone");
                break;
            }
        } else {
            break;
        }
    }
}

async fn send_timeout(tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>, ip: String) {
    let result = WsResponse::PingMonitor {
        ip,
        result: PingState::NoResponse,
    };
    if let Ok(payload) = encode_ws_message(&result) {
        if let Err(_) = tx.send(payload).await {
            info!("Channel is gone");
        }
    }
}

async fn send_alive(
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
    ip: String,
    ping_time: Duration,
    label: String,
) {
    let result = WsResponse::PingMonitor {
        ip,
        result: PingState::Ping {
            time_nanos: ping_time.as_nanos() as u64,
            label,
        },
    };
    if let Ok(payload) = encode_ws_message(&result) {
        if let Err(_) = tx.send(payload).await {
            info!("Channel is gone");
        }
    }
}

async fn ping(
    client: Client,
    addr: IpAddr,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
    label: String,
) {
    let payload = [0; 56];
    let mut pinger = client.pinger(addr, PingIdentifier(random())).await;
    pinger.timeout(Duration::from_secs(1));
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((IcmpPacket::V4(..), dur)) => {
            send_alive(tx, addr.to_string(), dur, label.clone()).await;
        }
        Ok((IcmpPacket::V6(..), dur)) => {
            send_alive(tx, addr.to_string(), dur, label.clone()).await;
        }
        _ => {
            send_timeout(tx, addr.to_string()).await;
            return;
        }
    }
}
