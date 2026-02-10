use std::sync::Arc;
use tracing::debug;

pub(crate) fn sqm_as_vec(config: &Arc<lqos_config::Config>) -> Vec<String> {
    config
        .queues
        .default_sqm
        .split(" ")
        .map(|s| s.to_string())
        .collect()
}

pub(crate) fn format_rate_for_tc(rate: u64) -> String {
    // Format a rate in Mbps for TC commands with smart unit selection.
    // - Rates >= 1000 Mbps use 'gbit'
    // - Rates >= 1 Mbps use 'mbit'
    // - Rates < 1 Mbps use 'kbit'
    if rate >= 1000 {
        format!("{:.1}gbit", rate as f64 / 1000.0)
    } else if rate >= 1 {
        format!("{:.1}mbit", rate as f64)
    } else {
        format!("{:.0}kbit", rate as f64 * 1000.0)
    }
}

pub(crate) fn format_rate_for_tc_f32(rate: f32) -> String {
    // Defensive formatting of a rate in Mbps for TC commands with smart unit selection.
    // Guards against NaN/Inf/negative/zero and clamps to sane bounds so `tc` always
    // receives a positive, finite value.

    // Lower bound: 0.01 Mbps (10 kbit) to avoid "0kbit" or rounding to zero.
    const MIN_RATE_MBPS: f32 = 0.01;
    // Upper bound: 10,000,000 Mbps (10 Tbps) — arbitrary but sane cap to avoid infinities.
    const MAX_RATE_MBPS: f32 = 10_000_000.0;

    let mut r = rate;

    // Handle NaN/Inf
    if !r.is_finite() {
        debug!(
            "format_rate_for_tc_f32: non-finite rate detected ({:?}); clamping to {} Mbps",
            rate, MIN_RATE_MBPS
        );
        r = MIN_RATE_MBPS;
    }

    // Clamp to bounds
    if r < MIN_RATE_MBPS {
        debug!(
            "format_rate_for_tc_f32: rate below minimum ({:?}); clamping to {} Mbps",
            r, MIN_RATE_MBPS
        );
        r = MIN_RATE_MBPS;
    } else if r > MAX_RATE_MBPS {
        debug!(
            "format_rate_for_tc_f32: rate above maximum ({:?}); clamping to {} Mbps",
            r, MAX_RATE_MBPS
        );
        r = MAX_RATE_MBPS;
    }

    // Format using thresholds
    if r >= 1000.0 {
        format!("{:.1}gbit", r as f64 / 1000.0)
    } else if r >= 1.0 {
        format!("{:.1}mbit", r as f64)
    } else {
        // < 1 Mbps: kbit, whole number to match tc expectations
        format!("{:.0}kbit", r as f64 * 1000.0)
    }
}

pub(crate) fn r2q(max_rate_in_mbps: u64) -> u64 {
    // Constants from Python implementation
    const MAX_R2Q: f64 = 60_000.0; // From LibreQoS.py

    // Convert rate in Mbps to bytes per second
    let max_rate_in_bytes_per_second = (max_rate_in_mbps * 125000) as f64;

    // Start with a default r2q value of 10
    let mut r2q = 10u64;

    // Calculate initial quantum using floating point division to match Python
    let mut quantum = max_rate_in_bytes_per_second / r2q as f64;

    // Increment r2q until quantum is below MAX_R2Q
    // This matches Python's behavior of comparing float values
    while quantum > MAX_R2Q {
        r2q += 1;
        quantum = max_rate_in_bytes_per_second / r2q as f64;
    }

    r2q
}

pub(crate) fn quantum(rate: u64, r2q: u64) -> String {
    // Constants from Python implementation
    const MIN_QUANTUM: u64 = 1522; // From LibreQoS.py

    // Convert rate in Mbps to bytes per second
    let rate_in_bytes_per_second = rate * 125000;

    // Calculate quantum value using the same logic as Python
    let quantum = std::cmp::max(MIN_QUANTUM, rate_in_bytes_per_second / r2q);

    // Format and return the quantum string
    quantum.to_string()
}

pub(crate) fn sqm_rate_fixup(rate: f32, config: &Arc<lqos_config::Config>) -> Vec<String> {
    // If we aren't using cake, just return the sqm string
    let sqm = &config.queues.default_sqm;
    if !sqm.starts_with("cake") || sqm.contains("rtt") {
        return sqm_as_vec(config);
    }

    let mut result = sqm_as_vec(config);

    // If we are using cake, we need to fixup the rate
    // Based on: 1 MTU is 1500 bytes, or 12,000 bits.
    // At 1 Mbps, (1,000 bits per ms) transmitting an MTU takes 12ms. Add 3ms for overhead, and we get 15ms.
    //    So 15ms divided by 5 (for 1%) multiplied by 100 yields 300ms.
    //    The same formula gives 180ms at 2Mbps
    //    140ms at 3Mbps
    //    120ms at 4Mbps
    // We don't change anything for rates above 4Mbps, as the default is 100ms.

    if rate <= 1.0 {
        result.push("rtt".to_string());
        result.push("300ms".to_string());
    } else if rate <= 2.0 {
        result.push("rtt".to_string());
        result.push("180ms".to_string());
    } else if rate <= 3.0 {
        result.push("rtt".to_string());
        result.push("140ms".to_string());
    } else if rate <= 4.0 {
        result.push("rtt".to_string());
        result.push("120ms".to_string());
    }
    result
}

/// Build SQM token vector for a circuit given an optional per-circuit override.
/// - None: use config default with cake low-rate RTT fixups (existing behavior)
/// - Some("fq_codel"): use fq_codel
/// - Some("cake"): use config default if it starts with "cake", otherwise fallback to
///   "cake diffserv4"; then apply low-rate RTT fixups.
pub(crate) fn sqm_tokens_for(
    rate: f32,
    config: &Arc<lqos_config::Config>,
    override_opt: &Option<String>,
) -> Vec<String> {
    match override_opt.as_deref() {
        None => {
            // Prefer fq_codel for very fast circuits if configured
            let threshold = config.queues.fast_queues_fq_codel.unwrap_or(1000.0) as f32;
            if rate >= threshold {
                return vec!["fq_codel".to_string()];
            }
            sqm_rate_fixup(rate, config)
        }
        Some("fq_codel") => vec!["fq_codel".to_string()],
        Some("cake") => {
            let default = &config.queues.default_sqm;
            let mut base = if default.starts_with("cake") {
                sqm_as_vec(config)
            } else {
                vec!["cake".to_string(), "diffserv4".to_string()]
            };
            // If RTT already specified, leave as-is; otherwise apply low-rate fixups
            let has_rtt = base.iter().any(|s| s == "rtt");
            if !has_rtt {
                // Mirror the thresholds used in sqm_rate_fixup
                if rate <= 1.0 {
                    base.push("rtt".to_string());
                    base.push("300ms".to_string());
                } else if rate <= 2.0 {
                    base.push("rtt".to_string());
                    base.push("180ms".to_string());
                } else if rate <= 3.0 {
                    base.push("rtt".to_string());
                    base.push("140ms".to_string());
                } else if rate <= 4.0 {
                    base.push("rtt".to_string());
                    base.push("120ms".to_string());
                }
            }
            base
        }
        Some(_) => sqm_rate_fixup(rate, config), // defensive fallback
    }
}
