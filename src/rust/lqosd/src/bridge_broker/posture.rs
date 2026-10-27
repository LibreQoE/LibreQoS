//! #4 (`docs/control_plane_simplification_card.md` §4c; the implementer
//! `docs/resolved_posture_spec.md`): the RESOLVED-POSTURE surface.
//!
//! Every §2 SEMANTIC knob with its RESOLVED value, emitted at boot AND via
//! `lqosd --print-resolved-posture`, so the DoD *".12 and jackrabbit have the
//! same resolved semantic posture"* is a mechanical diff and *"which drop-in
//! won?"* cannot recur. Tuning(103)/diagnostic knobs are NOT surfaced.
//!
//! The process env is FLAT ⇒ v1 emits `*_set` (explicit vs default); it does
//! NOT fake a `source=base|dropin:<file>` (that is v2, needing the unified
//! resolution).

/// The EXACT field set (the spec §2 + one `*_set` flag per value knob), SORTED.
/// The pin test asserts the resolved keys == this, so the surface cannot drift.
pub(crate) const POSTURE_FIELDS: &[&str] = &[
    "analytics_socket_kind",
    "analytics_socket_set",
    "artifact_boot",
    "artifact_boot_set",
    "baseline_max_circuits",
    "baseline_max_circuits_set",
    "build_rev",
    "bulk_generation_threshold",
    "bulk_generation_threshold_set",
    "config_file_set",
    "control_channel",
    "control_channel_set",
    "harness_boot",
    "harness_boot_set",
    "lkg_file_set",
    "placement_ownership",
    "placement_ownership_set",
    "queue_mode",
    "queue_mode_set",
    "transaction_file_set",
];

/// The emitting process's `side`.
pub(crate) const SIDE: &str = "lqosd";
/// The surface schema version.
pub(crate) const SCHEMA: u32 = 1;

/// The non-env inputs (so `resolve_posture` is unit-testable without a full
/// `lqos_config::Config`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PostureInput {
    /// `config.queues.queue_mode.is_observe()`.
    pub queue_mode_observe: bool,
    /// `config.dataplane_bridge.is_some()`.
    pub dataplane_bridge_present: bool,
}

fn env_set(key: &str) -> bool {
    std::env::var_os(key).is_some()
}
fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key).ok().as_deref() {
        Some("1") | Some("true") | Some("TRUE") | Some("True") => true,
        Some("0") | Some("false") | Some("FALSE") | Some("False") => false,
        _ => default,
    }
}
fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Resolve the §2 semantic posture (SORTED `(field, value)`).
pub(crate) fn resolve_posture(input: PostureInput) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::with_capacity(POSTURE_FIELDS.len());
    let mut push = |k: &'static str, v: String| out.push((k, v));

    push(
        "queue_mode",
        if input.queue_mode_observe {
            "observe".to_string()
        } else {
            "shape".to_string()
        },
    );
    push(
        "queue_mode_set",
        env_set("LIBREQOS_DPDK_QUEUE_MODE").to_string(),
    );
    push(
        "control_channel",
        env_bool("LQOS_CONTROL_CHANNEL", true).to_string(),
    );
    push(
        "control_channel_set",
        env_set("LQOS_CONTROL_CHANNEL").to_string(),
    );
    push(
        "placement_ownership",
        std::env::var("LIBREQOS_DPDK_PLACEMENT_OWNERSHIP").unwrap_or_else(|_| "auto".to_string()),
    );
    push(
        "placement_ownership_set",
        env_set("LIBREQOS_DPDK_PLACEMENT_OWNERSHIP").to_string(),
    );
    push(
        "transaction_file_set",
        env_set("LIBREQOS_DPDK_TRANSACTION_FILE").to_string(),
    );
    push(
        "config_file_set",
        env_set("LIBREQOS_DPDK_CONFIG_FILE").to_string(),
    );
    push(
        "artifact_boot",
        env_bool("LIBREQOS_DPDK_ARTIFACT_BOOT", false).to_string(),
    );
    push(
        "artifact_boot_set",
        env_set("LIBREQOS_DPDK_ARTIFACT_BOOT").to_string(),
    );
    push(
        "harness_boot",
        env_bool("LQOS_RIG_HARNESS_BOOT", false).to_string(),
    );
    push(
        "harness_boot_set",
        env_set("LQOS_RIG_HARNESS_BOOT").to_string(),
    );
    push(
        "analytics_socket_set",
        env_set("LIBREQOS_ANALYTICS_SOCKET").to_string(),
    );
    let kind = if env_set("LIBREQOS_ANALYTICS_SOCKET") {
        "env"
    } else if input.dataplane_bridge_present {
        "config"
    } else {
        "default"
    };
    push("analytics_socket_kind", kind.to_string());
    push("lkg_file_set", env_set("LIBREQOS_LKG_FILE").to_string());
    push(
        "baseline_max_circuits",
        lqos_dataplane_types::baseline_max_circuits().to_string(),
    );
    push(
        "baseline_max_circuits_set",
        env_set("LIBREQOS_DPDK_BASELINE_MAX_CIRCUITS").to_string(),
    );
    push(
        "bulk_generation_threshold",
        env_usize("LIBREQOS_DPDK_BULK_GENERATION_THRESHOLD", 8).to_string(),
    );
    push(
        "bulk_generation_threshold_set",
        env_set("LIBREQOS_DPDK_BULK_GENERATION_THRESHOLD").to_string(),
    );
    push(
        "build_rev",
        option_env!("LQOS_BUILD_REV").unwrap_or("unknown").to_string(),
    );

    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// Render the probe output: the header + sorted `field=value` lines, so
/// `| sort` is idempotent and the DoD diff is mechanical.
pub(crate) fn render_probe(posture: &[(&'static str, String)]) -> String {
    let mut lines = vec![
        "event=RESOLVED_POSTURE".to_string(),
        format!("schema={SCHEMA}"),
        format!("side={SIDE}"),
    ];
    for (k, v) in posture {
        lines.push(format!("{k}={v}"));
    }
    lines.sort();
    lines.join("\n") + "\n"
}

/// Emit the resolved posture as ONE structured `RESOLVED_POSTURE` record (the
/// sorted `field=value` body) — the boot surface an operator reads.
pub(crate) fn log_posture(posture: &[(&'static str, String)]) {
    let body = posture
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(
        event = "RESOLVED_POSTURE",
        schema = SCHEMA,
        side = SIDE,
        posture = %body,
        "resolved semantic posture (observe-before-collapse)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

 /// the implementer's spec §3: the field SET is PINNED — a dropped/added field fails
    /// here (the surface cannot silently drift from the spec).
    #[test]
    fn the_resolved_posture_field_set_matches_the_spec() {
        let input = PostureInput {
            queue_mode_observe: false,
            dataplane_bridge_present: false,
        };
        let keys: Vec<&str> = resolve_posture(input).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, POSTURE_FIELDS, "the field set must equal the spec");
        // SORTED (so the probe's `| sort` is idempotent + the diff is mechanical).
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "the emitted fields must be sorted");
    }

 /// the implementer N3: the field set is pinned to the SPEC FILE
    /// (`resolved_posture_fields.txt`, the authoritative list) — NOT to the
    /// impl's own `POSTURE_FIELDS` — so the surface cannot drift from the spec.
    #[test]
    fn the_field_set_is_pinned_to_the_spec_file() {
        let spec: Vec<&str> = include_str!("resolved_posture_fields.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let impl_keys: Vec<&str> = POSTURE_FIELDS.to_vec();
        assert_eq!(
            impl_keys, spec,
            "POSTURE_FIELDS must equal the authoritative spec list"
        );
    }

    /// The probe renders the header + the sorted field lines (diffable).
    #[test]
    fn the_probe_render_is_deterministic_and_carries_the_header() {
        let input = PostureInput {
            queue_mode_observe: true,
            dataplane_bridge_present: false,
        };
        let rendered = render_probe(&resolve_posture(input));
        assert!(rendered.contains("event=RESOLVED_POSTURE"));
        assert!(rendered.contains("schema=1"));
        assert!(rendered.contains("side=lqosd"));
        assert!(rendered.contains("queue_mode=observe"));
        // deterministic: rendering twice is identical.
        assert_eq!(rendered, render_probe(&resolve_posture(input)));
    }
}
