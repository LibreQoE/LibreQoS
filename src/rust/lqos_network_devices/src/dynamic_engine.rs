//! The dynamic-circuit decision engine.
//!
//! Every decision here is pure: configuration, time and host samples arrive as
//! values and the functions return decisions. The actor keeps the clock, the
//! catalog, the store writes and the hooks, so tests drive these decisions
//! without I/O, globals or a running actor.
//!
//! Three decisions live here:
//! - longest-prefix eligibility over `[[dynamic_circuits.ranges]]` (a
//!   `0.0.0.0/0` catch-all is an ordinary rule);
//! - promotion of a host from cumulatively counted bytes (sustained window,
//!   byte-rate threshold, counter sources, counter resets, per-range promotion
//!   cap);
//! - TTL expiry of store rows.

use crate::DynamicCircuit;
use lqos_config::DynamicCircuitRangeRule;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// The period the promotion cap counts over: `promotion_cap_per_range_per_minute`
/// is a per-minute cap.
const PROMOTION_CAP_WINDOW_MILLIS: u64 = 60_000;

/// Where one host's cumulative byte counters came from (u11).
///
/// The engine prices an interval only between samples from the SAME source: a
/// host carried by a range fallback and a host in the bridge's unmapped class are
/// counted by different accumulators, so subtracting one from the other would
/// fabricate a delta. A source change therefore REBASELINES the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivitySource {
    /// The bridge's unmapped (`CircuitId::best_effort`) host class.
    Unmapped,
    /// The numeric bridge circuit that carried this host's bytes. lqosd resolves
    /// the CIRCUIT by subscriber IP; the actor only needs the identity.
    Circuit(u64),
}

/// One host in a pushed promotion-candidate batch (u11 A4).
///
/// The batch carries CUMULATIVE bytes and the accumulator they came from, never
/// a rate: the engine owns the interval arithmetic (and its reset/rebaseline
/// rules), so lqosd keeps no per-host counters of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostActivity {
    /// The subscriber host.
    pub ip: IpAddr,
    /// Cumulative bytes (download + upload) the named source has counted.
    pub cumulative_bytes: u64,
    /// The counter accumulator these bytes came from.
    pub source: ActivitySource,
}

/// One host in a pushed batch, after the actor resolved it against the published
/// world. The engine decides from these fields alone.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ObservedHost {
    /// The host address.
    pub(crate) ip: IpAddr,
    /// Cumulative bytes (download + upload) the bridge has counted for this
    /// host, from `source`. A value BELOW the previous observation from the same
    /// source is a counter reset (host-row retention eviction), never a negative
    /// delta.
    pub(crate) cumulative_bytes: u64,
    /// The counter accumulator behind `cumulative_bytes`.
    pub(crate) source: ActivitySource,
    /// Not promotion material: an ordinary circuit already shapes this host (a
    /// provisioned device or an existing dynamic row), or an operator exclusion
    /// (`ip_ranges.ignore_subnets`) forbids promoting it. The actor owns the
    /// reason; the engine only needs the verdict.
    pub(crate) ineligible: bool,
}

/// The `[dynamic_circuits]` promotion knobs, as values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PromotionPolicy<'a> {
    /// `[[dynamic_circuits.ranges]]`, in configuration order.
    pub(crate) ranges: &'a [DynamicCircuitRangeRule],
    /// `promotion_min_bytes_per_second`.
    pub(crate) min_bytes_per_second: u64,
    /// `promotion_window_seconds`.
    pub(crate) window_seconds: u64,
    /// `promotion_cap_per_range_per_minute`.
    pub(crate) cap_per_range_per_minute: u32,
}

/// A promotion the actor should provision.
#[derive(Clone, Debug)]
pub(crate) struct Promotion {
    /// The host to provision.
    pub(crate) ip: IpAddr,
    /// The most specific rule that matched it: its rates and `attach_to` are the
    /// plan.
    pub(crate) rule: DynamicCircuitRangeRule,
}

/// The most specific configured rule whose prefix contains `ip`.
///
/// Longest-prefix match: a `/0` catch-all applies only where nothing more
/// specific does, and a promoted `/32` shadows it. Two rules claiming the same
/// prefix make the answer ambiguous, and an ambiguous host is not eligible —
/// guessing between two rate plans is worse than not promoting (Oracle ruling
/// 1: reject equal-prefix ownership ambiguity).
pub(crate) fn eligible_rule(
    rules: &[DynamicCircuitRangeRule],
    ip: IpAddr,
) -> Option<&DynamicCircuitRangeRule> {
    let mut best: Option<&DynamicCircuitRangeRule> = None;
    let mut best_prefix = 0u8;
    let mut ambiguous = false;
    for rule in rules {
        if !rule.ip_range.contains(ip) {
            continue;
        }
        let prefix = rule.ip_range.netmask();
        if best.is_none() || prefix > best_prefix {
            best = Some(rule);
            best_prefix = prefix;
            ambiguous = false;
        } else if prefix == best_prefix {
            ambiguous = true;
        }
    }
    if ambiguous { None } else { best }
}

/// The sustained-activity windows and the per-rule promotion cap.
///
/// One window per host: a host's activity is judged against the most specific
/// rule that matches it, so a rule change restarts the window instead of
/// carrying another range's evidence across. A window is also restarted when the
/// COUNTER SOURCE changes, the counter decreases, or the observation clock goes
/// backwards (u11 A5): none of those is an interval the engine may price.
#[derive(Default)]
pub(crate) struct PromotionEngine {
    windows: HashMap<IpAddr, ActivityWindow>,
    promotions: HashMap<String, VecDeque<u64>>,
}

struct ActivityWindow {
    /// The rule the host matched when the window opened.
    rule_name: String,
    /// The counter accumulator the window's deltas are measured against.
    source: ActivitySource,
    /// The observation that opened the window (millis).
    first_millis: u64,
    /// The last observation folded into the window (millis).
    last_millis: u64,
    /// The cumulative-byte baseline at `last_millis`.
    last_bytes: u64,
}

impl ActivityWindow {
    fn opened(
        rule_name: String,
        source: ActivitySource,
        observed_at_millis: u64,
        cumulative_bytes: u64,
    ) -> Self {
        Self {
            rule_name,
            source,
            first_millis: observed_at_millis,
            last_millis: observed_at_millis,
            last_bytes: cumulative_bytes,
        }
    }
}

impl PromotionEngine {
    /// Folds one bounded batch of host observations into the windows and returns
    /// the promotions that became due.
    ///
    /// `observed_at_millis` is the OBSERVATION time of the batch, not the time
    /// this ran, so queued batches are priced at the interval they describe.
    ///
    /// The first sighting of a host is a baseline, never evidence; a promotion
    /// needs a full `window_seconds` of above-threshold intervals with no gap
    /// longer than the window, no counter reset in between, and a free slot in
    /// the rule's per-minute cap.
    pub(crate) fn observe(
        &mut self,
        observed_at_millis: u64,
        policy: &PromotionPolicy<'_>,
        observed: &[ObservedHost],
    ) -> Vec<Promotion> {
        let window_millis = policy.window_seconds.saturating_mul(1000);
        let mut promotions = Vec::new();

        for host in observed {
            let Some(rule) = eligible_rule(policy.ranges, host.ip) else {
                self.windows.remove(&host.ip);
                continue;
            };
            if host.ineligible {
                self.windows.remove(&host.ip);
                continue;
            }
            let rule_name = rule.name.trim().to_string();
            let Some(window) = self.windows.get_mut(&host.ip) else {
                self.windows.insert(
                    host.ip,
                    ActivityWindow::opened(
                        rule_name,
                        host.source,
                        observed_at_millis,
                        host.cumulative_bytes,
                    ),
                );
                continue;
            };
            if window.rule_name != rule_name {
                *window = ActivityWindow::opened(
                    rule_name,
                    host.source,
                    observed_at_millis,
                    host.cumulative_bytes,
                );
                continue;
            }
            if window.source != host.source {
                // u11 A5: a different accumulator is a different measurement.
                // Price nothing across the change; start a fresh window.
                *window = ActivityWindow::opened(
                    rule_name,
                    host.source,
                    observed_at_millis,
                    host.cumulative_bytes,
                );
                continue;
            }
            if observed_at_millis < window.last_millis {
                // u11 A5: the observation clock went backwards (a bridge restart
                // resets its boot-relative stamp). That is not an interval.
                *window = ActivityWindow::opened(
                    window.rule_name.clone(),
                    window.source,
                    observed_at_millis,
                    host.cumulative_bytes,
                );
                continue;
            }

            let elapsed_millis = observed_at_millis.saturating_sub(window.last_millis);
            if elapsed_millis == 0 {
                // Same instant: there is no interval to price, so leave the
                // baseline alone and let the next observation carry both.
                continue;
            }
            if host.cumulative_bytes < window.last_bytes {
                // Counter reset: the interval's delta is unknown. Restart from
                // this baseline instead of trusting a negative rate.
                *window = ActivityWindow::opened(
                    window.rule_name.clone(),
                    window.source,
                    observed_at_millis,
                    host.cumulative_bytes,
                );
                continue;
            }

            let delta = host.cumulative_bytes - window.last_bytes;
            window.last_bytes = host.cumulative_bytes;
            window.last_millis = observed_at_millis;
            if elapsed_millis > window_millis {
                // A gap longer than the window is not sustained activity.
                window.first_millis = observed_at_millis;
                continue;
            }
            let bytes_per_second = delta.saturating_mul(1000) / elapsed_millis;
            if bytes_per_second < policy.min_bytes_per_second {
                // One below-threshold interval breaks the window: a trickle
                // between bursts is not sustained traffic.
                window.first_millis = observed_at_millis;
                continue;
            }
            if observed_at_millis.saturating_sub(window.first_millis) < window_millis {
                continue;
            }

            let stamps = self.promotions.entry(window.rule_name.clone()).or_default();
            while stamps.front().is_some_and(|stamp| {
                observed_at_millis.saturating_sub(*stamp) >= PROMOTION_CAP_WINDOW_MILLIS
            }) {
                stamps.pop_front();
            }
            if stamps.len() as u32 >= policy.cap_per_range_per_minute {
                // Keep the window: it promotes as soon as the cap frees a slot.
                continue;
            }
            stamps.push_back(observed_at_millis);
            promotions.push(Promotion {
                ip: host.ip,
                rule: rule.clone(),
            });
            self.windows.remove(&host.ip);
        }

        self.windows.retain(|_, window| {
            observed_at_millis.saturating_sub(window.last_millis) <= window_millis
        });
        promotions
    }
}

/// The store rows whose TTL has expired.
pub(crate) fn expired_dynamic_circuit_ids(
    circuits: &[DynamicCircuit],
    now_unix: u64,
    ttl_seconds: u64,
) -> Vec<String> {
    circuits
        .iter()
        .filter_map(|circuit| {
            let circuit_id = circuit.shaped.circuit_id.trim();
            if circuit_id.is_empty() {
                return None;
            }
            // REV-C1: TTL expiry needs EVIDENCE. A stamp of 0 means liveness was
            // never established for this row (a clock-not-ready boot, or a
            // degraded checkpoint the loader could not restore); expiring it
            // would delete the whole overlay the moment the clock returns.
            if circuit.last_seen_unix == 0 {
                return None;
            }
            let age_seconds = now_unix.saturating_sub(circuit.last_seen_unix);
            (age_seconds > ttl_seconds).then_some(circuit_id.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ip_network::IpNetwork;

    fn rule(name: &str, range: &str) -> DynamicCircuitRangeRule {
        DynamicCircuitRangeRule {
            name: name.to_string(),
            ip_range: range.parse::<IpNetwork>().expect("range parses"),
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
            attach_to: String::new(),
        }
    }

    fn observed(ip: &str, cumulative_bytes: u64) -> ObservedHost {
        carried_by(ip, cumulative_bytes, ActivitySource::Unmapped)
    }

    /// One observation from an explicit counter source (u11).
    fn carried_by(ip: &str, cumulative_bytes: u64, source: ActivitySource) -> ObservedHost {
        ObservedHost {
            ip: ip.parse().expect("ip parses"),
            cumulative_bytes,
            source,
            ineligible: false,
        }
    }

    fn policy<'a>(ranges: &'a [DynamicCircuitRangeRule]) -> PromotionPolicy<'a> {
        PromotionPolicy {
            ranges,
            min_bytes_per_second: 1024,
            window_seconds: 5,
            cap_per_range_per_minute: 10,
        }
    }

    #[test]
    fn eligibility_is_longest_prefix_and_the_zero_prefix_is_a_catch_all() {
        let ranges = vec![
            rule("default", "0.0.0.0/0"),
            rule("broad", "10.0.0.0/8"),
            rule("specific", "10.1.2.0/24"),
            rule("host", "10.1.2.99/32"),
        ];

        let name = |ip: &str| {
            eligible_rule(&ranges, ip.parse().expect("ip parses")).map(|rule| rule.name.as_str())
        };
        assert_eq!(name("192.0.2.1"), Some("default"), "the /0 catch-all covers");
        assert_eq!(name("10.2.0.1"), Some("broad"));
        assert_eq!(name("10.1.2.3"), Some("specific"));
        assert_eq!(
            name("10.1.2.99"),
            Some("host"),
            "a /32 beats the /24 it lives in"
        );
        assert_eq!(
            name("2001:db8::1"),
            None,
            "an IPv4 catch-all is not an IPv6 one"
        );
    }

    #[test]
    fn equal_prefix_rules_are_ambiguous_and_never_eligible() {
        let ranges = vec![rule("first", "10.0.0.0/8"), rule("second", "10.0.0.0/8")];
        assert!(
            eligible_rule(&ranges, "10.1.1.1".parse().expect("ip parses")).is_none(),
            "two rules claiming one prefix must not silently pick a rate plan"
        );
        assert!(
            eligible_rule(&ranges, "192.0.2.1".parse().expect("ip parses")).is_none(),
            "an unmatched address is no more eligible than an ambiguous one"
        );
    }

    #[test]
    fn promotion_needs_a_full_window_of_above_threshold_intervals() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let policy = policy(&ranges);
        let mut engine = PromotionEngine::default();

        // Baseline, then one second of 4096 B/s: 4 of the 5 required seconds.
        assert!(engine.observe(0, &policy, &[observed("10.9.0.7", 0)]).is_empty());
        for second in 1..5u64 {
            assert!(
                engine
                    .observe(second * 1000, &policy, &[observed("10.9.0.7", second * 4096)])
                    .is_empty(),
                "second {second} is before the window closes"
            );
        }
        let promotions = engine.observe(5000, &policy, &[observed("10.9.0.7", 5 * 4096)]);
        assert_eq!(promotions.len(), 1, "the sustained window promotes once");
        assert_eq!(promotions[0].ip, "10.9.0.7".parse::<IpAddr>().expect("ip"));
        assert_eq!(promotions[0].rule.name, "default");
        assert!(
            engine.observe(6000, &policy, &[observed("10.9.0.7", 6 * 4096)]).is_empty(),
            "a promoted host leaves no window behind"
        );
    }

    #[test]
    fn below_threshold_and_zero_byte_intervals_never_promote() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let policy = policy(&ranges);
        let mut engine = PromotionEngine::default();

        engine.observe(0, &policy, &[observed("10.9.0.8", 0)]);
        // 100 B/s is under the 1024 B/s bar.
        assert!(
            engine
                .observe(1000, &policy, &[observed("10.9.0.8", 100)])
                .is_empty()
        );
        // A flat counter is 0 B/s: presence alone is never activity.
        for second in 2..8u64 {
            assert!(
                engine
                    .observe(second * 1000, &policy, &[observed("10.9.0.8", 100)])
                    .is_empty(),
                "second {second} carries no new bytes"
            );
        }
    }

    #[test]
    fn a_counter_reset_rebaselines_instead_of_promoting() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let policy = policy(&ranges);
        let mut engine = PromotionEngine::default();

        // Sustained long enough to promote, except the counter resets at t=3s.
        engine.observe(0, &policy, &[observed("10.9.0.9", 0)]);
        engine.observe(1000, &policy, &[observed("10.9.0.9", 4096)]);
        engine.observe(2000, &policy, &[observed("10.9.0.9", 8192)]);
        assert!(
            engine
                .observe(3000, &policy, &[observed("10.9.0.9", 100)])
                .is_empty(),
            "a counter reset is not a promotion"
        );
        // The window restarted at the reset: it needs a full window from there.
        for second in 4..8u64 {
            assert!(
                engine
                    .observe(
                        second * 1000,
                        &policy,
                        &[observed("10.9.0.9", 100 + (second - 3) * 4096)]
                    )
                    .is_empty(),
                "second {second} is before the post-reset window closes"
            );
        }
        let promotions =
            engine.observe(8000, &policy, &[observed("10.9.0.9", 100 + 5 * 4096)]);
        assert_eq!(promotions.len(), 1, "the post-reset window promotes");
    }

    /// u11 A5: the batch may switch a host from the fallback's accumulator to the
    /// unmapped class (or to another circuit). Those are different counters, so
    /// the interval across the change is not evidence — the window restarts and
    /// the NEXT full window promotes from the new source alone.
    #[test]
    fn a_source_change_rebaselines_the_window_instead_of_pricing_it() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let mut policy = policy(&ranges);
        policy.window_seconds = 2;
        let mut engine = PromotionEngine::default();

        // Two seconds of a fallback-carried host: a full window, but the
        // promotion below never happens because the source changes first.
        engine.observe(
            0,
            &policy,
            &[carried_by("10.9.4.1", 0, ActivitySource::Circuit(7))],
        );
        assert!(
            engine
                .observe(
                    1000,
                    &policy,
                    &[carried_by("10.9.4.1", 4096, ActivitySource::Circuit(7))]
                )
                .is_empty(),
            "one second is not the two-second window"
        );
        // The host starts arriving as unmapped (the fallback was removed): its
        // cumulative counter jumps, and a source change must not be priced.
        assert!(
            engine
                .observe(
                    2000,
                    &policy,
                    &[carried_by("10.9.4.1", 900_000, ActivitySource::Unmapped)]
                )
                .is_empty(),
            "the interval spanning two accumulators is not activity"
        );
        // A full window FROM THE NEW SOURCE promotes exactly once.
        assert!(
            engine
                .observe(
                    3000,
                    &policy,
                    &[carried_by("10.9.4.1", 904_096, ActivitySource::Unmapped)]
                )
                .is_empty(),
            "the new source's window is one second old"
        );
        let promotions = engine.observe(
            4000,
            &policy,
            &[carried_by("10.9.4.1", 908_192, ActivitySource::Unmapped)],
        );
        assert_eq!(promotions.len(), 1, "the new source's full window promotes");
        assert_eq!(promotions[0].ip, "10.9.4.1".parse::<IpAddr>().expect("ip"));
    }

    /// u11 A5: a bridge restart resets the observation clock (a boot-relative
    /// stamp). A backwards clock is not an interval: the window rebaselines and
    /// the post-rollback window is what can promote.
    #[test]
    fn a_clock_rollback_rebaselines_instead_of_freezing_the_window() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let mut policy = policy(&ranges);
        policy.window_seconds = 2;
        let mut engine = PromotionEngine::default();

        engine.observe(500_000, &policy, &[observed("10.9.5.1", 0)]);
        engine
            .observe(501_000, &policy, &[observed("10.9.5.1", 4096)]);
        // Clock rollback: the baseline restarts at the new stamp.
        assert!(
            engine
                .observe(1_000, &policy, &[observed("10.9.5.1", 8_192)])
                .is_empty(),
            "the interval spanning the rollback is not priced"
        );
        // Two seconds from the rollback's stamp is a full window again.
        engine.observe(2_000, &policy, &[observed("10.9.5.1", 12_288)]);
        let promotions = engine.observe(3_000, &policy, &[observed("10.9.5.1", 16_384)]);
        assert_eq!(
            promotions.len(),
            1,
            "the post-rollback window promotes (a frozen window would never)"
        );
    }

    #[test]
    fn a_gap_longer_than_the_window_breaks_sustained_activity() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let policy = policy(&ranges);
        let mut engine = PromotionEngine::default();

        engine.observe(0, &policy, &[observed("10.9.0.10", 0)]);
        engine.observe(1000, &policy, &[observed("10.9.0.10", 4096)]);
        assert!(
            engine
                .observe(20_000, &policy, &[observed("10.9.0.10", 24_000)])
                .is_empty(),
            "a 19 s gap is a new activity episode, not a sustained one"
        );
        assert!(
            engine
                .observe(21_000, &policy, &[observed("10.9.0.10", 28_096)])
                .is_empty(),
            "the gap restarted the window"
        );
    }

    #[test]
    fn the_promotion_cap_is_per_rule_per_minute() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let mut policy = policy(&ranges);
        policy.cap_per_range_per_minute = 1;
        policy.window_seconds = 1;
        let mut engine = PromotionEngine::default();

        let first = observed("10.9.1.1", 0);
        let second = observed("10.9.1.2", 0);
        engine.observe(0, &policy, &[first, second]);
        // Both are sustained at t=1 s; the cap admits exactly one (batch order).
        let admitted = engine.observe(
            1000,
            &policy,
            &[observed("10.9.1.1", 4096), observed("10.9.1.2", 4096)],
        );
        assert_eq!(admitted.len(), 1, "the cap admits one host per minute");
        assert_eq!(admitted[0].ip, "10.9.1.1".parse::<IpAddr>().expect("ip"));

        // The capped host keeps reporting above-threshold traffic and waits.
        for second in 2..=60u64 {
            assert!(
                engine
                    .observe(
                        second * 1000,
                        &policy,
                        &[observed("10.9.1.2", (second - 1) * 4096)]
                    )
                    .is_empty(),
                "second {second} is still inside the cap window"
            );
        }
        // One minute after the admitted promotion the slot is free.
        let promoted = engine.observe(61_000, &policy, &[observed("10.9.1.2", 60 * 4096)]);
        assert_eq!(promoted.len(), 1, "the waiting host promotes on a free slot");
        assert_eq!(promoted[0].ip, "10.9.1.2".parse::<IpAddr>().expect("ip"));
    }

    #[test]
    fn ineligible_hosts_are_never_promoted_even_inside_a_catch_all() {
        let ranges = vec![rule("default", "0.0.0.0/0")];
        let mut policy = policy(&ranges);
        policy.window_seconds = 1;
        let mut engine = PromotionEngine::default();

        let eligible_host = observed("10.9.2.1", 0);
        let mut blocked_host = observed("10.9.2.2", 0);
        blocked_host.ineligible = true;
        engine.observe(0, &policy, &[eligible_host, blocked_host]);
        let promotions = engine.observe(
            1000,
            &policy,
            &[observed("10.9.2.1", 4096), {
                let mut host = observed("10.9.2.2", 4096);
                host.ineligible = true;
                host
            }],
        );
        assert_eq!(promotions.len(), 1);
        assert_eq!(
            promotions[0].ip,
            "10.9.2.1".parse::<IpAddr>().expect("ip"),
            "the provisioned host inside the same catch-all stays unshaped by promotion"
        );
    }

    #[test]
    fn a_narrower_rule_restarts_the_window_instead_of_inheriting_it() {
        let catch_all = vec![rule("default", "0.0.0.0/0")];
        let narrower = vec![
            rule("default", "0.0.0.0/0"),
            rule("narrow", "10.9.3.0/24"),
        ];
        let wide = policy(&catch_all);
        let mut after_reload = policy(&narrower);
        after_reload.window_seconds = 5;
        let mut engine = PromotionEngine::default();

        engine.observe(0, &wide, &[observed("10.9.3.1", 0)]);
        assert!(
            engine
                .observe(4000, &wide, &[observed("10.9.3.1", 4 * 4096)])
                .is_empty()
        );
        // A reload adds a rule that is now the most specific match. The window
        // restarts under the new rule: the catch-all's four seconds are not
        // evidence for the narrower plan.
        for second in 5..10u64 {
            assert!(
                engine
                    .observe(
                        second * 1000,
                        &after_reload,
                        &[observed("10.9.3.1", second * 4096)]
                    )
                    .is_empty(),
                "second {second} is before the narrower window closes"
            );
        }
        let promotions =
            engine.observe(10_000, &after_reload, &[observed("10.9.3.1", 10 * 4096)]);
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].rule.name, "narrow");
    }

    #[test]
    fn expiry_needs_a_real_stamp_and_a_strictly_older_row() {
        let row = |circuit_id: &str, last_seen_unix: u64| DynamicCircuit {
            shaped: lqos_config::ShapedDevice {
                circuit_id: circuit_id.to_string(),
                ..Default::default()
            },
            last_seen_unix,
        };
        let circuits = vec![row("alive", 900), row("expired", 100), row("unproven", 0)];
        assert_eq!(
            expired_dynamic_circuit_ids(&circuits, 1_000, 300),
            vec!["expired".to_string()],
        );
    }
}
