//! LTS2 (Insight) per-flow export — the ONE `bridge_analytics` tick consumer
//! that maps the recent bridge flow rows to
//! `lts2_sys::one_way_flow` / `lts2_sys::two_way_flow`.
//!
//! The retired kernel finished-flow feed used to produce these; the K3-B
//! deletion (`72b424df9`) left the Insight per-flow export with **no caller**.
//! `bridge_analytics::flow_history` is now the ONE flow source (review ,
//! carve-out sweep #1), so the export is re-homed here.
//!
//! The mapping mirrors the retired `finished_flows::enqueue` exactly:
//!   * **one-way** = one direction carries ZERO bytes  → `OneWayFlow`;
//!   * **two-way** = both directions carry bytes        → `TwoWayFlow`;
//!   * **completion-gated + retained once** (the netflow consumer's
//!     discipline — the ring re-presents the same row across ticks);
//!   * `start_time`/`end_time` are UNIX **seconds**. The bridge rows carry
//!     **MILLISECONDS SINCE BOOT** (`analytics_server.rs` — TSC /
//!     `last_seen_ns`, NO epoch offset), so they are converted here via
//!     `lqos_utils::unix_time::boot_time_nanos_to_unix_now` (the retired
//!     producer's conversion — v2 parity). A bare `/1000` yields
//!     seconds-since-boot, which the Insight server cannot place in its window;
//!   * `circuit_hash` = the shaped-devices catalog LPM lookup for the
//!     subscriber IP (the retired producer's fallback), 0 when unknown;
//!   * the `analytics_ipc::FlowId` supplies local/remote IPs, protocol, and the
//!     subscriber/remote ports.

use analytics_ipc::{FlowRow, FlowState};
use lqos_utils::unix_time::boot_time_nanos_to_unix_now;
use parking_lot::Mutex;
use std::net::IpAddr;
use std::sync::OnceLock;

use crate::lts2_sys::shared_types::{OneWayFlow, TwoWayFlow};
use crate::netflow::FlowIdRetention;

/// Bounded once-only retention (the netflow exporter's cap).
const RETENTION_CAPACITY: usize = 200_000;

static RETAINED: OnceLock<Mutex<FlowIdRetention>> = OnceLock::new();

fn retained() -> &'static Mutex<FlowIdRetention> {
    RETAINED.get_or_init(|| Mutex::new(FlowIdRetention::new(RETENTION_CAPACITY)))
}

/// The retired producer only exported FINISHED flows; the bridge ring also
/// carries active rows, so gate on the completion states.
fn is_completion(flow: &FlowRow) -> bool {
    matches!(
        flow.state,
        FlowState::Fin | FlowState::Reset | FlowState::Idle
    )
}

/// The circuit-hash for a flow's SUBSCRIBER ip (the retired producer's
/// fallback chain ended at the shaped-devices LPM lookup). 0 = unknown.
fn circuit_hash_for(subscriber_ip: IpAddr) -> i64 {
    lqos_network_devices::shaped_devices_catalog()
        .circuit_hash_for_ip(&lqos_utils::XdpIpAddress::from_ip(subscriber_ip))
        .unwrap_or(0)
}

/// The per-direction summary RTT in MICROSECONDS (the `TwoWayFlow.rtt` unit) —
/// the median of the direction's raw samples.  `None` when the direction's ring
/// is empty: an ABSENT leg, not a zero (`lqos-74t.2.5`); the caller then carries
/// the connection's other leg (a DELIBERATE product divergence — see `build_flow`).
fn median_us(samples: &[u32]) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2] as f32)
}

/// The two LTS2 wire shapes this consumer can build (the retired predicate).
/// The wire types carry only `Serialize`/`Deserialize`, so this enum does too.
enum BuiltFlow {
    OneWay(OneWayFlow),
    TwoWay(TwoWayFlow),
}

/// The Insight `retransmit_times_*` array for one direction.
///
/// Insight computes the retransmit % as
/// `array_length(retransmit_times_down, 1) / packets_down * 100` — it reads
/// ONLY the array LENGTH, never the values. The bridge keeps the real times in
/// a bounded 8-slot ring (`stats.rs` `TcpDirectionState::retransmit_times`),
/// so the ring alone would UNDER-COUNT. The array is therefore PADDED up to
/// the counted total (`tcp_retransmits_down/up`): the recorded times (converted
/// boot-ns → unix seconds by the existing `boot_time_nanos_to_unix_now`) come
/// first, then padding repeats the last recorded time — or the flow's last-seen
/// unix second when the ring is empty but the count is not. The resulting
/// LENGTH == the count by construction.
fn retransmit_times(recorded_ns: &[u64], count: u64, last_seen_millis: u64) -> Vec<i64> {
    let mut times: Vec<i64> = recorded_ns
        .iter()
        .map(|&timestamp_ns| boot_time_nanos_to_unix_now(timestamp_ns).unwrap_or(0) as i64)
        .collect();
    let target = usize::try_from(count).unwrap_or(usize::MAX);
    let fallback = times.last().copied().unwrap_or_else(|| {
        boot_time_nanos_to_unix_now(last_seen_millis.saturating_mul(1_000_000)).unwrap_or(0) as i64
    });
    // Exactly `count` entries: pads up (the ring under-count) and truncates
    // down (only possible after a counter reset, so the length never exceeds
    // the count).
    times.resize(target, fallback);
    times
}

/// Maps ONE completed flow row to the LTS2 wire type. One-way when either
/// direction carries ZERO bytes (the retired `enqueue` predicate); else
/// two-way. PURE (the shaped-devices hash lookup is the only exterior read).
fn build_flow(row: &FlowRow) -> BuiltFlow {
    let down = row.actual.download_bytes;
    let up = row.actual.upload_bytes;
    // The bridge rows are MILLIS SINCE BOOT; Insight needs UNIX seconds. Convert
    // via the retired producer's helper (v2 parity) — a bare `/1000` is wrong.
    let start_time =
        boot_time_nanos_to_unix_now(row.first_seen_millis.saturating_mul(1_000_000)).unwrap_or(0);
    let end_time =
        boot_time_nanos_to_unix_now(row.last_seen_millis.saturating_mul(1_000_000)).unwrap_or(0);
    let circuit_hash = circuit_hash_for(row.id.subscriber_ip);
    if down == 0 || up == 0 {
        BuiltFlow::OneWay(OneWayFlow {
            start_time,
            end_time,
            local_ip: row.id.subscriber_ip,
            remote_ip: row.id.remote_ip,
            protocol: row.id.protocol,
            dst_port: row.id.remote_port,
            src_port: row.id.subscriber_port,
            bytes: down.saturating_add(up),
            circuit_hash,
        })
    } else {
        // The bridge feed carries retransmit COUNTS and a bounded ring of the
        // real retransmit TIMES. Insight uses `array_length` as the numerator,
        // so the array is padded to the counted total (never the 8-cap).
        BuiltFlow::TwoWay(TwoWayFlow {
            start_time,
            end_time,
            local_ip: row.id.subscriber_ip,
            remote_ip: row.id.remote_ip,
            protocol: row.id.protocol,
            dst_port: row.id.remote_port,
            src_port: row.id.subscriber_port,
            bytes_down: down,
            bytes_up: up,
            retransmit_times_down: retransmit_times(
                &row.retransmit_times_down_ns,
                row.tcp_retransmits_down,
                row.last_seen_millis,
            ),
            retransmit_times_up: retransmit_times(
                &row.retransmit_times_up_ns,
                row.tcp_retransmits_up,
                row.last_seen_millis,
            ),
            rtt: {
                // lqos-74t.2.5 (Root cause C) — a DELIBERATE PRODUCT DIVERGENCE
                // (DRIVER ruling, card §7): an empty per-direction ring is
                // ABSENT, not a false 0.0 — a 0.0 trips Insight's QoO gate
                // (`rtt_ms <= 0.0 -> None`, qoo_combined.rs:120) and DROPS the
                // direction.  We choose BETTER QoO coverage on single-direction
                // flows: a leg with no samples carries the OTHER leg's (then the
                // merged union's) median, so the direction keeps a POSITIVE RTT.
                // This is NOT v2 parity — v2's per-direction `summary_rtt_nanos`
                // is `unwrap_or(0)`, and v2's LTS export ALSO ships `[x, 0.0]`
                // (the gate then drops the leg).  The wire stays `[f32; 2]`: the
                // Insight ingestor types `TwoWayFlow.rtt` as `[f32; 2]`, so a
                // `null` would fail deserialization — hence NO Insight change.
                // Only a flow with NO samples anywhere ships 0.0 (nothing to carry).
                let down = median_us(&row.rtt_samples_down_us);
                let up = median_us(&row.rtt_samples_up_us);
                let merged = median_us(&row.rtt_samples_us);
                [
                    down.or(up).or(merged).unwrap_or(0.0),
                    up.or(down).or(merged).unwrap_or(0.0),
                ]
            },
            circuit_hash,
            packets_down: Some(row.sent_pkts_down as i64),
            packets_up: Some(row.sent_pkts_up as i64),
        })
    }
}

/// Build + send one flow to the LTS2 client.
fn export_flow(row: &FlowRow) {
    match build_flow(row) {
        BuiltFlow::OneWay(flow) => {
            if let Err(error) = crate::lts2_sys::one_way_flow(flow) {
                tracing::debug!("Failed to send one-way flow to LTS2: {error:?}");
            }
        }
        BuiltFlow::TwoWay(flow) => {
            if let Err(error) = crate::lts2_sys::two_way_flow(flow) {
                tracing::debug!("Failed to send two-way flow to LTS2: {error:?}");
            }
        }
    }
}

/// The tick consumer: export each recent flow ONCE at its completion, gated by
/// the LTS2 `gather_stats` arm (the retired producer's gate) AND the actor's
/// active state. Called from `bridge_analytics`'s poll tick after the
/// `flow_history` fold, alongside the netflow exporter.
pub(crate) fn export_recent_flows(flows: &[FlowRow]) {
    let enabled = lqos_config::load_config()
        .map(|config| config.long_term_stats.gather_stats)
        .unwrap_or(false)
        && super::is_active();
    if !enabled {
        return;
    }
    let mut retention = retained().lock();
    for flow in flows {
        if !is_completion(flow) {
            continue;
        }
        if !retention.insert(flow.id.clone()) {
            continue;
        }
        export_flow(flow);
    }
}

/// The RING-PATH export (`lqos-74t.2.x` Q3): `seq` is the once-only authority,
/// so BYPASS `FlowIdRetention` (a legitimate second completion with the same
/// FlowId inside the retention window must not be dropped). Returns `false`
/// when any send failed (the caller must NOT ack that batch).
pub(crate) fn export_ring_batch(flows: &[FlowRow]) -> bool {
    let enabled = lqos_config::load_config()
        .map(|config| config.long_term_stats.gather_stats)
        .unwrap_or(false)
        && super::is_active();
    if !enabled {
        return true;
    }
    let mut ok = true;
    for flow in flows {
        if !is_completion(flow) {
            continue;
        }
        let sent = match build_flow(flow) {
            BuiltFlow::OneWay(flow) => crate::lts2_sys::one_way_flow(flow).is_ok(),
            BuiltFlow::TwoWay(flow) => crate::lts2_sys::two_way_flow(flow).is_ok(),
        };
        if !sent {
            ok = false;
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{DirectionalCounters, FlowId};

    fn row(down: u64, up: u64, rtt_down: Vec<u32>, rtt_up: Vec<u32>) -> FlowRow {
        FlowRow {
            id: FlowId {
                circuit_id: 7,
                flow_hash: 42,
                subscriber_ip: "192.0.2.10".parse().unwrap(),
                remote_ip: "198.51.100.20".parse().unwrap(),
                subscriber_port: 51_234,
                remote_port: 443,
                protocol: 6,
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            // BOOT-RELATIVE, as the bridge ACTUALLY emits (5 s / 10 s since
            // boot) — a unix-looking fixture would mask the time-base bug.
            first_seen_millis: 5_000,
            last_seen_millis: 9_999,
            rtt_samples_down_us: rtt_down,
            rtt_samples_up_us: rtt_up,
            ..Default::default()
        }
    }

    /// The RETIRED predicate: one direction with zero bytes ⇒ OneWayFlow,
    /// carrying `bytes = down + up`, the BOOT-RELATIVE→UNIX time conversion,
    /// and the FlowId port/IP mapping.
    #[test]
    fn zero_direction_maps_to_one_way_flow() {
        let flow = build_flow(&row(0, 2_048, vec![], vec![]));
        let BuiltFlow::OneWay(one) = flow else {
            panic!("a zero direction must be one-way");
        };
        // NON-VACUOUS: a boot-relative 5 s input must emerge as UNIX seconds
        // (a bare /1000 would yield 5 — the live bug).
        assert!(
            one.start_time >= 1_600_000_000,
            "start_time must be UNIX seconds, not seconds-since-boot: {}",
            one.start_time
        );
        assert_eq!(
            one.end_time - one.start_time,
            4,
            "the 5s→10s duration survives the conversion"
        );
        assert_eq!(one.bytes, 2_048);
        assert_eq!(one.local_ip, "192.0.2.10".parse::<IpAddr>().unwrap());
        assert_eq!(one.remote_ip, "198.51.100.20".parse::<IpAddr>().unwrap());
        assert_eq!(one.protocol, 6);
        assert_eq!(one.src_port, 51_234);
        assert_eq!(one.dst_port, 443);
    }

    /// Both directions carry bytes ⇒ TwoWayFlow with the per-direction bytes,
    /// the MEDIAN summary RTT in micros, and `retransmit_times_*` whose LENGTH
    /// equals the counted retransmits (Insight's numerator).
    #[test]
    fn both_directions_map_to_two_way_flow_with_median_rtt() {
        let mut flow = row(1_000, 500, vec![10, 30, 20], vec![7]);
        flow.tcp_retransmits_down = 3;
        flow.tcp_retransmits_up = 1;
        // BOOT-RELATIVE ns (6 s / 7 s / 8 s since boot), as the bridge emits them.
        flow.retransmit_times_down_ns = vec![6_000_000_000, 7_000_000_000, 7_500_000_000];
        flow.retransmit_times_up_ns = vec![8_000_000_000];
        let flow = build_flow(&flow);
        let BuiltFlow::TwoWay(two) = flow else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(two.bytes_down, 1_000);
        assert_eq!(two.bytes_up, 500);
        assert!(
            two.start_time >= 1_600_000_000,
            "start_time must be UNIX seconds, not seconds-since-boot: {}",
            two.start_time
        );
        assert_eq!(two.end_time - two.start_time, 4);
        assert_eq!(two.rtt, [20.0, 7.0], "median micros per direction");
        // Oracle: the exported LENGTH equals the bridge's counted retransmits.
        assert_eq!(two.retransmit_times_down.len(), 3);
        assert_eq!(two.retransmit_times_up.len(), 1);
        // The recorded times convert boot-ns → unix seconds (not seconds-since-boot).
        assert!(
            two.retransmit_times_down.iter().all(|&t| t >= 1_600_000_000),
            "times must be unix seconds: {:?}",
            two.retransmit_times_down
        );
    }

    /// The DRIVER ruling (B): the exported array LENGTH must equal the counted
    /// total even when the bridge's bounded 8-slot ring under-holds — Insight
    /// reads only `array_length`, so the array is PADDED to the count.
    #[test]
    fn retransmit_times_pad_to_the_counted_total() {
        let mut flow = row(1_000, 500, vec![], vec![]);
        flow.tcp_retransmits_down = 10;
        // The ring keeps only its 8 newest stays; the counter is the truth.
        flow.retransmit_times_down_ns = (1..=8).map(|s| s * 1_000_000_000).collect();
        flow.tcp_retransmits_up = 0;
        let BuiltFlow::TwoWay(two) = build_flow(&flow) else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(
            two.retransmit_times_down.len(),
            10,
            "the exported length == the counted total (the 8-cap must not under-count)"
        );
        assert!(
            two.retransmit_times_down.iter().all(|&t| t >= 1_600_000_000),
            "padding repeats a real unix second: {:?}",
            two.retransmit_times_down
        );
        assert!(
            two.retransmit_times_up.is_empty(),
            "zero count ⇒ empty array (no fabricated entries)"
        );
    }

    /// A count with an EMPTY ring (the times were never recorded for this
    /// direction) still exports `count` entries, padded from the flow's
    /// last-seen second — the length tracks the COUNT, not the ring.
    #[test]
    fn retransmit_times_with_empty_ring_still_match_the_count() {
        let mut flow = row(1_000, 500, vec![], vec![]);
        flow.tcp_retransmits_down = 2;
        let BuiltFlow::TwoWay(two) = build_flow(&flow) else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(two.retransmit_times_down.len(), 2);
        assert!(
            two.retransmit_times_down.iter().all(|&t| t >= 1_600_000_000),
            "the last-seen fallback is a unix second: {:?}",
            two.retransmit_times_down
        );
    }

    /// Median picks the upper-middle sample for an even count (stable).
    #[test]
    fn median_us_is_upper_middle_for_even_counts() {
        assert_eq!(median_us(&[4, 1, 3, 2]), Some(3.0));
        assert_eq!(median_us(&[]), None, "an empty ring is ABSENT, not 0.0");
    }

    /// `lqos-74t.2.5` (Root cause C): a leg with no samples carries the other
    /// leg's RTT (v2 parity) — it must NOT ship a false 0.0 (which trips
    /// Insight's QoO gate `rtt_ms <= 0.0 -> None` and drops the direction).
    #[test]
    fn empty_leg_carries_the_other_legs_rtt() {
        // DOWN has samples, UP is empty ⇒ both directions carry the down RTT.
        let flow = row(2_000, 2_000, vec![10, 30, 20], vec![]);
        let BuiltFlow::TwoWay(two) = build_flow(&flow) else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(two.rtt, [20.0, 20.0], "the empty up leg carries the down RTT");

        // Both legs empty ⇒ the merged union is the fallback.
        let mut flow = row(2_000, 2_000, vec![], vec![]);
        flow.rtt_samples_us = vec![40, 50];
        let BuiltFlow::TwoWay(two) = build_flow(&flow) else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(two.rtt, [50.0, 50.0], "the merged union carries both legs");

        // NO samples anywhere ⇒ 0.0 (nothing to carry).
        let flow = row(2_000, 2_000, vec![], vec![]);
        let BuiltFlow::TwoWay(two) = build_flow(&flow) else {
            panic!("two non-zero directions must be two-way");
        };
        assert_eq!(two.rtt, [0.0, 0.0]);
    }
}
