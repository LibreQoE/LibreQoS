use std::sync::Arc;

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
    // Format a rate in Mbps for TC commands with smart unit selection.
    // - Rates >= 1000 Mbps use 'gbit'
    // - Rates >= 1 Mbps use 'mbit'
    // - Rates < 1 Mbps use 'kbit'
    if rate >= 1000.0 {
        format!("{:.1}gbit", rate as f64 / 1000.0)
    } else if rate >= 1.0 {
        format!("{:.1}mbit", rate as f64)
    } else {
        format!("{:.0}kbit", rate as f64 * 1000.0)
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
        result.push("300".to_string());
    } else if rate <= 2.0 {
        result.push("rtt".to_string());
        result.push("180".to_string());
    } else if rate <= 3.0 {
        result.push("rtt".to_string());
        result.push("140".to_string());
    } else if rate <= 4.0 {
        result.push("rtt".to_string());
        result.push("120".to_string());
    }
    result
}
