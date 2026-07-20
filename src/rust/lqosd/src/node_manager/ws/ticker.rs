use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::node_manager::ws::publish_subscribe::PubSub;
use futures_util::FutureExt;
use lqos_bus::{BusReply, BusRequest};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::panic::AssertUnwindSafe;
use tokio::join;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, timeout};
use tracing::{debug, warn};
mod asn_top;
mod bakery;
mod cadence;
mod circuit_capacity;
mod endpoint_latlon;
mod executive_dashboard_summary;
mod flow_counter;
mod flow_endpoints;
pub(crate) mod ipstats_conversion;
mod network_tree;
mod network_tree_lite;
mod queue_stats_total;
mod retransmits;
mod rtt_histogram;
mod stormguard;
pub mod system_info;
mod throughput;
mod top_10;
mod top_flows;
mod tree_capacity;
mod tree_summary;
mod tree_summary_l2;
mod treeguard;

use crate::system_stats::SystemStats;
pub use network_tree::all_circuits;

const TICKER_TASK_TIMEOUT: Duration = Duration::from_millis(5_500);
const INTERNAL_BUS_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const INTERNAL_BUS_REQUEST_COOLDOWN: Duration = Duration::from_secs(3);
static INTERNAL_BUS_REQUEST_COOLDOWNS: Lazy<Mutex<HashMap<&'static str, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn internal_bus_request_is_cooling_down(context: &'static str) -> bool {
    let now = Instant::now();
    let mut cooldowns = INTERNAL_BUS_REQUEST_COOLDOWNS.lock();
    match cooldowns.get(context).copied() {
        Some(until) if until > now => {
            debug!(
                ticker = context,
                remaining_ms = until.duration_since(now).as_millis(),
                "Skipping ticker bus request during cooldown"
            );
            true
        }
        Some(_) => {
            cooldowns.remove(context);
            false
        }
        None => false,
    }
}

fn start_internal_bus_request_cooldown(
    context: &'static str,
    request_kind: &'static str,
    reason: &'static str,
    cooldown_duration: Duration,
) {
    INTERNAL_BUS_REQUEST_COOLDOWNS
        .lock()
        .insert(context, Instant::now() + cooldown_duration);
    warn!(
        ticker = context,
        request_kind,
        reason,
        cooldown_ms = cooldown_duration.as_millis(),
        "Ticker bus request failed; cooling down"
    );
}

pub(super) async fn request_internal_bus(
    context: &'static str,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
    request: BusRequest,
) -> Option<BusReply> {
    request_internal_bus_with_timeout(
        context,
        bus_tx,
        request,
        INTERNAL_BUS_REQUEST_TIMEOUT,
        INTERNAL_BUS_REQUEST_COOLDOWN,
    )
    .await
}

async fn request_internal_bus_with_timeout(
    context: &'static str,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
    request: BusRequest,
    timeout_duration: Duration,
    cooldown_duration: Duration,
) -> Option<BusReply> {
    if internal_bus_request_is_cooling_down(context) {
        return None;
    }

    let request_kind = request.kind();
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    match timeout(timeout_duration, bus_tx.send((tx, request))).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            start_internal_bus_request_cooldown(
                context,
                request_kind,
                "send_failed",
                cooldown_duration,
            );
            return None;
        }
        Err(_) => {
            start_internal_bus_request_cooldown(
                context,
                request_kind,
                "send_timeout",
                cooldown_duration,
            );
            return None;
        }
    }

    match timeout(timeout_duration, rx).await {
        Ok(Ok(replies)) => Some(replies),
        Ok(Err(_)) => {
            start_internal_bus_request_cooldown(
                context,
                request_kind,
                "reply_channel_closed",
                cooldown_duration,
            );
            None
        }
        Err(_) => {
            start_internal_bus_request_cooldown(
                context,
                request_kind,
                "reply_timeout",
                cooldown_duration,
            );
            None
        }
    }
}

async fn ticker_with_timeout<T>(name: &'static str, fut: impl std::future::Future<Output = T>) {
    let result = timeout(TICKER_TASK_TIMEOUT, AssertUnwindSafe(fut).catch_unwind()).await;
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(panic)) => warn!(
            ticker = name,
            panic = panic_payload_to_string(&panic),
            "Ticker panicked"
        ),
        Err(_) => warn!(ticker = name, "Ticker timed out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_bus::BusResponse;

    const TEST_SEND_FAILED_CONTEXT: &str = "test_send_failed_context";
    const TEST_COOLDOWN_CONTEXT: &str = "test_cooldown_context";
    const TEST_REPLY_CLOSED_CONTEXT: &str = "test_reply_closed_context";
    const TEST_REPLY_TIMEOUT_CONTEXT: &str = "test_reply_timeout_context";
    const TEST_SUCCESS_CONTEXT: &str = "test_success_context";
    const TEST_EXPIRED_COOLDOWN_CONTEXT: &str = "test_expired_cooldown_context";

    #[tokio::test]
    async fn internal_bus_request_cools_down_when_send_fails() {
        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_SEND_FAILED_CONTEXT);

        let (bus_tx, bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);
        drop(bus_rx);

        assert!(
            request_internal_bus(TEST_SEND_FAILED_CONTEXT, bus_tx, BusRequest::Ping)
                .await
                .is_none()
        );
        assert!(
            INTERNAL_BUS_REQUEST_COOLDOWNS
                .lock()
                .contains_key(TEST_SEND_FAILED_CONTEXT)
        );

        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_SEND_FAILED_CONTEXT);
    }

    #[tokio::test]
    async fn internal_bus_request_skips_send_during_cooldown() {
        INTERNAL_BUS_REQUEST_COOLDOWNS.lock().insert(
            TEST_COOLDOWN_CONTEXT,
            Instant::now() + INTERNAL_BUS_REQUEST_COOLDOWN,
        );

        let (bus_tx, mut bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);

        assert!(
            request_internal_bus(TEST_COOLDOWN_CONTEXT, bus_tx, BusRequest::Ping)
                .await
                .is_none()
        );
        assert!(bus_rx.try_recv().is_err());

        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_COOLDOWN_CONTEXT);
    }

    #[tokio::test]
    async fn internal_bus_request_returns_replies() {
        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_SUCCESS_CONTEXT);

        let (bus_tx, mut bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);
        let responder = tokio::spawn(async move {
            let Some((reply_tx, request)) = bus_rx.recv().await else {
                panic!("test bus request missing");
            };
            assert_eq!(request.kind(), "Ping");
            reply_tx
                .send(BusReply {
                    responses: vec![BusResponse::Ack],
                })
                .expect("test receiver is alive");
        });

        let response = request_internal_bus_with_timeout(
            TEST_SUCCESS_CONTEXT,
            bus_tx,
            BusRequest::Ping,
            Duration::from_millis(50),
            Duration::from_millis(20),
        )
        .await
        .expect("bus reply should arrive");

        responder.await.unwrap();
        assert_eq!(response.responses, vec![BusResponse::Ack]);
        assert!(
            !INTERNAL_BUS_REQUEST_COOLDOWNS
                .lock()
                .contains_key(TEST_SUCCESS_CONTEXT)
        );
    }

    #[tokio::test]
    async fn internal_bus_request_sends_after_cooldown_expires() {
        INTERNAL_BUS_REQUEST_COOLDOWNS.lock().insert(
            TEST_EXPIRED_COOLDOWN_CONTEXT,
            Instant::now() - Duration::from_millis(1),
        );

        let (bus_tx, mut bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);
        let responder = tokio::spawn(async move {
            let Some((reply_tx, request)) = bus_rx.recv().await else {
                panic!("test bus request missing after cooldown expiry");
            };
            assert_eq!(request.kind(), "Ping");
            reply_tx
                .send(BusReply {
                    responses: vec![BusResponse::Ack],
                })
                .expect("test receiver is alive");
        });

        let response = request_internal_bus_with_timeout(
            TEST_EXPIRED_COOLDOWN_CONTEXT,
            bus_tx,
            BusRequest::Ping,
            Duration::from_millis(50),
            Duration::from_millis(20),
        )
        .await
        .expect("expired cooldown should allow request");

        responder.await.unwrap();
        assert_eq!(response.responses, vec![BusResponse::Ack]);
        assert!(
            !INTERNAL_BUS_REQUEST_COOLDOWNS
                .lock()
                .contains_key(TEST_EXPIRED_COOLDOWN_CONTEXT)
        );
    }

    #[tokio::test]
    async fn internal_bus_request_cools_down_when_reply_channel_closes() {
        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_REPLY_CLOSED_CONTEXT);

        let (bus_tx, mut bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);
        let responder = tokio::spawn(async move {
            let Some((reply_tx, request)) = bus_rx.recv().await else {
                panic!("test bus request missing");
            };
            assert_eq!(request.kind(), "Ping");
            drop(reply_tx);
        });

        assert!(
            request_internal_bus_with_timeout(
                TEST_REPLY_CLOSED_CONTEXT,
                bus_tx,
                BusRequest::Ping,
                Duration::from_millis(5),
                Duration::from_millis(20),
            )
            .await
            .is_none()
        );
        responder.await.unwrap();
        assert!(
            INTERNAL_BUS_REQUEST_COOLDOWNS
                .lock()
                .contains_key(TEST_REPLY_CLOSED_CONTEXT)
        );

        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_REPLY_CLOSED_CONTEXT);
    }

    #[tokio::test]
    async fn internal_bus_request_cools_down_when_reply_times_out() {
        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_REPLY_TIMEOUT_CONTEXT);

        let (bus_tx, mut bus_rx) =
            tokio::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>(1);
        let responder = tokio::spawn(async move {
            let Some((reply_tx, request)) = bus_rx.recv().await else {
                panic!("test bus request missing");
            };
            assert_eq!(request.kind(), "Ping");
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = reply_tx.send(BusReply {
                responses: vec![BusResponse::Ack],
            });
        });

        assert!(
            request_internal_bus_with_timeout(
                TEST_REPLY_TIMEOUT_CONTEXT,
                bus_tx,
                BusRequest::Ping,
                Duration::from_millis(5),
                Duration::from_millis(20),
            )
            .await
            .is_none()
        );
        responder.await.unwrap();
        assert!(
            INTERNAL_BUS_REQUEST_COOLDOWNS
                .lock()
                .contains_key(TEST_REPLY_TIMEOUT_CONTEXT)
        );

        INTERNAL_BUS_REQUEST_COOLDOWNS
            .lock()
            .remove(TEST_REPLY_TIMEOUT_CONTEXT);
    }
}

fn panic_payload_to_string(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    debug!("Starting channel tickers");
    one_second_cadence(channels.clone(), bus_tx.clone(), system_usage_tx.clone()).await;
}

async fn one_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await; // Once per second
        channels.update_living_channel_list().await;
        join!(
            ticker_with_timeout("cadence", cadence::cadence(channels.clone())),
            ticker_with_timeout(
                "throughput",
                throughput::throughput(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "rtt_histogram",
                rtt_histogram::rtt_histo(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "flow_counter",
                flow_counter::flow_count(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_10_downloaders",
                top_10::top_10_downloaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_10_uploaders",
                top_10::top_10_uploaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "worst_10_downloaders",
                top_10::worst_10_downloaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "worst_10_retransmit",
                top_10::worst_10_retransmit(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_flows_bytes",
                top_flows::top_flows_bytes(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_flows_rate",
                top_flows::top_flows_rate(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "asn_top",
                asn_top::asn_top(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "endpoints_by_country",
                flow_endpoints::endpoints_by_country(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "ether_protocols",
                flow_endpoints::ether_protocols(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "ip_protocols",
                flow_endpoints::ip_protocols(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "flow_duration",
                flow_endpoints::flow_duration(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "endpoint_latlon",
                endpoint_latlon::endpoint_latlon(channels.clone())
            ),
            ticker_with_timeout(
                "tree_summary",
                tree_summary::tree_summary(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "tree_summary_l2",
                tree_summary_l2::tree_summary_l2(channels.clone())
            ),
            ticker_with_timeout(
                "all_subscribers",
                network_tree::all_subscribers(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "queue_stats_totals",
                queue_stats_total::queue_stats_totals(channels.clone())
            ),
            ticker_with_timeout(
                "network_tree",
                network_tree::network_tree(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "network_tree_lite",
                network_tree_lite::network_tree_lite(channels.clone())
            ),
            ticker_with_timeout(
                "circuit_capacity",
                circuit_capacity::circuit_capacity(channels.clone())
            ),
            ticker_with_timeout(
                "tree_capacity",
                tree_capacity::tree_capacity(channels.clone())
            ),
            ticker_with_timeout(
                "cpu_info",
                system_info::cpu_info(channels.clone(), system_usage_tx.clone())
            ),
            ticker_with_timeout(
                "ram_info",
                system_info::ram_info(channels.clone(), system_usage_tx.clone())
            ),
            ticker_with_timeout(
                "tcp_retransmits",
                retransmits::tcp_retransmits(channels.clone())
            ),
            ticker_with_timeout(
                "stormguard",
                stormguard::stormguard_ticker(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout("bakery_status", bakery::bakery_status(channels.clone())),
            ticker_with_timeout("bakery_activity", bakery::bakery_activity(channels.clone())),
            ticker_with_timeout(
                "treeguard_status",
                treeguard::treeguard_status(channels.clone())
            ),
            ticker_with_timeout(
                "treeguard_activity",
                treeguard::treeguard_activity(channels.clone())
            ),
            ticker_with_timeout(
                "executive_dashboard_summary",
                executive_dashboard_summary::executive_dashboard_summary(channels.clone())
            ),
        );

        channels.clean().await;
    }
}
