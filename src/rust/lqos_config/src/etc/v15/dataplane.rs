//! The `[dataplane]` section — the SINGLE SOURCE for the ISP-editable dataplane
//! facts (card `docs/lqos_conf_single_source_card.md` §3).
//!
//! `deny_unknown_fields` is scoped to THIS struct ONLY (the global v15 stays
//! tolerant): a removed/unknown `[dataplane]` key fails loudly (oracle 4),
//! never silently ignored.
//!
//! Every key names a NEW fact + a named consumer + a behavior test (the `#12`
//! gate B/C). `download_mbps`/`upload_mbps` are the bandwidth single source the
//! bridge reads (the `bridge.env` `DOWNLOAD_MBPS`/`UPLOAD_MBPS` duplication
//! dies here). `subscriber_count`/`root_ceil_mbps` were DROPPED (they named no
//! fact §0's keys do not already carry).

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// The `[dataplane]` operator facts. Strict: an unknown field is a hard parse
/// error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Allocative)]
#[serde(deny_unknown_fields)]
pub struct DataplaneConfig {
    /// Download bandwidth (Mbps). The bridge resolves the same fact the control
    /// plane does (the one source). A **decimal** is accepted (`12.5` ⇒
    /// 12_500_000 bps) — the SAME exact-decimal form the engine's `[dataplane]`
    /// parser accepts (`bridge_rs::config::parse_mbps`), so the control plane
    /// and the engine never diverge on a sub-Mbps root rate.
    pub download_mbps: f64,
    /// Upload bandwidth (Mbps); see `download_mbps`.
    pub upload_mbps: f64,
}

#[cfg(test)]
mod tests {
    use super::DataplaneConfig;

    #[test]
    fn dataplane_parses_the_two_bandwidth_keys() {
        let parsed: DataplaneConfig =
            toml::from_str("download_mbps = 900\nupload_mbps = 450\n").expect("parses");
        assert_eq!(parsed.download_mbps, 900.0);
        assert_eq!(parsed.upload_mbps, 450.0);
    }

    #[test]
    fn dataplane_accepts_a_decimal_mbps_like_the_engine() {
        let parsed: DataplaneConfig =
            toml::from_str("download_mbps = 12.5\nupload_mbps = 0.5\n").expect("parses");
        assert_eq!(parsed.download_mbps, 12.5);
        assert_eq!(parsed.upload_mbps, 0.5);
    }

    #[test]
    fn dataplane_rejects_an_unknown_field_loudly() {
        // Oracle 4: a removed/unknown key is rejected, never silently ignored.
        let error = toml::from_str::<DataplaneConfig>(
            "download_mbps = 900\nupload_mbps = 450\nsubscriber_count = 12\n",
        )
        .expect_err("an unknown [dataplane] key must be rejected");
        assert!(
            error.to_string().contains("subscriber_count"),
            "the error must name the unknown key: {error}"
        );
    }
}
