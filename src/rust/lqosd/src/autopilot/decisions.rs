//! Pure decision logic for Autopilot.
//!
//! Functions in this module should be pure: they must not perform I/O, mutate globals,
//! or have side effects beyond returning a decision.

use crate::autopilot::state::{CircuitSqmState, CircuitState, LinkState, LinkVirtualState};
use lqos_config::{
    AutopilotCircuitsConfig, AutopilotCpuConfig, AutopilotCpuMode, AutopilotLinksConfig,
    AutopilotQooConfig,
};
use lqos_utils::units::DownUpOrder;

/// A virtualization decision for a managed link/node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkVirtualDecision {
    /// No state change is required.
    NoChange,
    /// Set the node's desired virtualization state.
    Set(LinkVirtualState),
}

/// A per-circuit SQM switching decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CircuitSqmDecision {
    /// Desired download direction SQM state change, if any.
    pub down: Option<CircuitSqmState>,
    /// Desired upload direction SQM state change, if any.
    pub up: Option<CircuitSqmState>,
}

/// Returns true if CPU pressure permits taking CPU-saving actions.
///
/// This function is pure: it has no side effects.
fn cpu_allows_saving(cpu: &AutopilotCpuConfig, cpu_max_pct: Option<u8>) -> bool {
    match cpu.mode {
        AutopilotCpuMode::CpuAware => cpu_max_pct.is_some_and(|pct| pct >= cpu.cpu_high_pct),
        AutopilotCpuMode::TrafficRttOnly => true,
    }
}

/// Returns true if CPU headroom calls for reverting CPU-saving actions.
///
/// This function is pure: it has no side effects.
fn cpu_calls_for_revert(cpu: &AutopilotCpuConfig, cpu_max_pct: Option<u8>) -> bool {
    match cpu.mode {
        AutopilotCpuMode::CpuAware => cpu_max_pct.is_some_and(|pct| pct <= cpu.cpu_low_pct),
        AutopilotCpuMode::TrafficRttOnly => false,
    }
}

/// Returns true if QoO (when available) is below the configured threshold for any direction.
///
/// This function is pure: it has no side effects.
fn qoo_below_threshold(qoo_cfg: &AutopilotQooConfig, qoo: DownUpOrder<Option<f32>>) -> bool {
    if !qoo_cfg.enabled {
        return false;
    }

    let below = |v: Option<f32>| v.is_some_and(|score| score < qoo_cfg.min_score);
    below(qoo.down) || below(qoo.up)
}

/// Returns true if the given timestamp is within the configured dwell time window.
///
/// This function is pure: it has no side effects.
fn in_dwell_window(now_unix: u64, last_change_unix: Option<u64>, dwell_minutes: u64) -> bool {
    let Some(last) = last_change_unix else {
        return false;
    };
    let dwell_secs = dwell_minutes.saturating_mul(60);
    now_unix.saturating_sub(last) < dwell_secs
}

/// Returns true if the number of recent state changes exceeds a configured limit.
///
/// This function is pure: it has no side effects.
fn rate_limited(recent_changes: usize, max_changes_per_hour: u32) -> bool {
    if max_changes_per_hour == 0 {
        return true;
    }
    recent_changes >= max_changes_per_hour as usize
}

/// Decide whether to virtualize/unvirtualize a managed node.
///
/// This function is pure: it has no side effects.
pub fn decide_link_virtualization(
    now_unix: u64,
    allowlisted: bool,
    cpu_max_pct: Option<u8>,
    cpu_cfg: &AutopilotCpuConfig,
    links_cfg: &AutopilotLinksConfig,
    qoo_cfg: &AutopilotQooConfig,
    rtt_missing: bool,
    qoo: DownUpOrder<Option<f32>>,
    util_ewma_pct: DownUpOrder<f64>,
    sustained_idle: bool,
    state: &LinkState,
) -> LinkVirtualDecision {
    if !allowlisted {
        return LinkVirtualDecision::NoChange;
    }

    if in_dwell_window(
        now_unix,
        state.last_change_unix,
        links_cfg.min_state_dwell_minutes,
    ) {
        return LinkVirtualDecision::NoChange;
    }

    if rate_limited(
        state.recent_changes_unix.len(),
        links_cfg.max_link_changes_per_hour,
    ) {
        return LinkVirtualDecision::NoChange;
    }

    let qoo_bad = qoo_below_threshold(qoo_cfg, qoo);

    match state.desired {
        LinkVirtualState::Physical => {
            if !cpu_allows_saving(cpu_cfg, cpu_max_pct) {
                return LinkVirtualDecision::NoChange;
            }
            if sustained_idle && !rtt_missing && !qoo_bad {
                LinkVirtualDecision::Set(LinkVirtualState::Virtual)
            } else {
                LinkVirtualDecision::NoChange
            }
        }
        LinkVirtualState::Virtual => {
            let util_high = util_ewma_pct.down >= links_cfg.unvirtualize_util_pct as f64
                || util_ewma_pct.up >= links_cfg.unvirtualize_util_pct as f64;
            if util_high || rtt_missing || qoo_bad {
                LinkVirtualDecision::Set(LinkVirtualState::Physical)
            } else {
                LinkVirtualDecision::NoChange
            }
        }
    }
}

/// Decide whether to switch a managed circuit's SQM profile per direction.
///
/// This function is pure: it has no side effects.
pub fn decide_circuit_sqm(
    now_unix: u64,
    allowlisted: bool,
    cpu_max_pct: Option<u8>,
    cpu_cfg: &AutopilotCpuConfig,
    circuits_cfg: &AutopilotCircuitsConfig,
    qoo_cfg: &AutopilotQooConfig,
    rtt_missing: bool,
    qoo: DownUpOrder<Option<f32>>,
    state: &CircuitState,
) -> CircuitSqmDecision {
    if !allowlisted {
        return CircuitSqmDecision::default();
    }

    if !circuits_cfg.switching_enabled {
        return CircuitSqmDecision::default();
    }

    let mut decision = CircuitSqmDecision::default();

    let decide_direction = |dir_qoo: Option<f32>,
                            dir_state: &crate::autopilot::state::CircuitDirectionState|
     -> Option<CircuitSqmState> {
        if in_dwell_window(
            now_unix,
            dir_state.last_change_unix,
            circuits_cfg.min_switch_dwell_minutes,
        ) {
            return None;
        }

        if rate_limited(
            dir_state.recent_changes_unix.len(),
            circuits_cfg.max_switches_per_hour,
        ) {
            return None;
        }

        let qoo_bad = if qoo_cfg.enabled {
            dir_qoo.is_some_and(|score| score < qoo_cfg.min_score)
        } else {
            false
        };

        match dir_state.desired {
            CircuitSqmState::Cake => {
                if cpu_allows_saving(cpu_cfg, cpu_max_pct) && !rtt_missing && !qoo_bad {
                    Some(CircuitSqmState::FqCodel)
                } else {
                    None
                }
            }
            CircuitSqmState::FqCodel => {
                if rtt_missing || qoo_bad || cpu_calls_for_revert(cpu_cfg, cpu_max_pct) {
                    Some(CircuitSqmState::Cake)
                } else {
                    None
                }
            }
        }
    };

    if circuits_cfg.independent_directions {
        decision.down = decide_direction(qoo.down, &state.down);
        decision.up = decide_direction(qoo.up, &state.up);
    } else {
        // Non-independent: decide using worst-direction QoO, apply to both.
        let worst_qoo = match (qoo.down, qoo.up) {
            (Some(d), Some(u)) => Some(d.min(u)),
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };
        let proposed = decide_direction(worst_qoo, &state.down);
        if let Some(s) = proposed {
            decision.down = Some(s);
            decision.up = Some(s);
        }
    }

    decision
}

/// Formats an SQM override token from per-direction desired states.
///
/// This function is pure: it has no side effects.
pub fn format_directional_sqm_override(down: CircuitSqmState, up: CircuitSqmState) -> String {
    let down_s = match down {
        CircuitSqmState::Cake => "cake",
        CircuitSqmState::FqCodel => "fq_codel",
    };
    let up_s = match up {
        CircuitSqmState::Cake => "cake",
        CircuitSqmState::FqCodel => "fq_codel",
    };
    format!("{down_s}/{up_s}")
}

/// Parses an SQM override token into per-direction SQM states.
///
/// The token may be a single value (applies to both directions) or a `down/up` token.
/// Empty and `"none"` tokens map to `None` for that direction.
///
/// This function is pure: it has no side effects.
pub fn parse_directional_sqm_override(token: &str) -> DownUpOrder<Option<CircuitSqmState>> {
    fn parse_one(t: &str) -> Option<CircuitSqmState> {
        let t = t.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("none") {
            return None;
        }
        if t.eq_ignore_ascii_case("cake") {
            return Some(CircuitSqmState::Cake);
        }
        if t.eq_ignore_ascii_case("fq_codel") {
            return Some(CircuitSqmState::FqCodel);
        }
        None
    }

    let token = token.trim();
    if token.is_empty() {
        return DownUpOrder { down: None, up: None };
    }

    if let Some((down, up)) = token.split_once('/') {
        return DownUpOrder {
            down: parse_one(down),
            up: parse_one(up),
        };
    }

    let v = parse_one(token);
    DownUpOrder { down: v, up: v }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::state::{CircuitDirectionState, CircuitState, LinkState};
    use std::collections::VecDeque;

    #[test]
    fn link_decision_requires_allowlist() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = LinkState::default();
        let decision = decide_link_virtualization(
            1000,
            false,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 0.5, up: 0.5 },
            true,
            &state,
        );
        assert_eq!(decision, LinkVirtualDecision::NoChange);
    }

    #[test]
    fn link_virtualizes_when_idle_and_cpu_high() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = LinkState::default();
        let decision = decide_link_virtualization(
            1000,
            true,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 1.0, up: 1.0 },
            true,
            &state,
        );
        assert_eq!(decision, LinkVirtualDecision::Set(LinkVirtualState::Virtual));
    }

    #[test]
    fn link_does_not_virtualize_when_cpu_low() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = LinkState::default();
        let decision = decide_link_virtualization(
            1000,
            true,
            Some(10),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 1.0, up: 1.0 },
            true,
            &state,
        );
        assert_eq!(decision, LinkVirtualDecision::NoChange);
    }

    #[test]
    fn link_unvirtualizes_on_util_spike() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = LinkState::default();
        state.desired = LinkVirtualState::Virtual;

        let decision = decide_link_virtualization(
            1000,
            true,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 10.0, up: 1.0 },
            false,
            &state,
        );
        assert_eq!(
            decision,
            LinkVirtualDecision::Set(LinkVirtualState::Physical)
        );
    }

    #[test]
    fn link_unvirtualizes_when_rtt_missing() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = LinkState::default();
        state.desired = LinkVirtualState::Virtual;

        let decision = decide_link_virtualization(
            1000,
            true,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            true,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 1.0, up: 1.0 },
            true,
            &state,
        );
        assert_eq!(
            decision,
            LinkVirtualDecision::Set(LinkVirtualState::Physical)
        );
    }

    #[test]
    fn link_dwell_time_blocks_changes() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = LinkState::default();
        state.last_change_unix = Some(1000 - 60);

        let decision = decide_link_virtualization(
            1000,
            true,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 1.0, up: 1.0 },
            true,
            &state,
        );
        assert_eq!(decision, LinkVirtualDecision::NoChange);
    }

    #[test]
    fn link_rate_limit_blocks_changes() {
        let cpu = AutopilotCpuConfig::default();
        let links = AutopilotLinksConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = LinkState::default();
        state.recent_changes_unix = VecDeque::from(vec![1, 2, 3, 4]);

        let decision = decide_link_virtualization(
            1000,
            true,
            Some(90),
            &cpu,
            &links,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(100.0),
                up: Some(100.0),
            },
            DownUpOrder { down: 1.0, up: 1.0 },
            true,
            &state,
        );
        assert_eq!(decision, LinkVirtualDecision::NoChange);
    }

    #[test]
    fn circuit_decision_requires_allowlist() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = CircuitState::default();
        let decision = decide_circuit_sqm(
            1000,
            false,
            Some(90),
            &cpu,
            &circuits,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(90.0),
                up: Some(90.0),
            },
            &state,
        );
        assert_eq!(decision, CircuitSqmDecision::default());
    }

    #[test]
    fn circuit_downgrades_under_cpu_pressure() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = CircuitState::default();
        let decision = decide_circuit_sqm(
            1000,
            true,
            Some(90),
            &cpu,
            &circuits,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(90.0),
                up: Some(90.0),
            },
            &state,
        );
        assert_eq!(decision.down, Some(CircuitSqmState::FqCodel));
        assert_eq!(decision.up, Some(CircuitSqmState::FqCodel));
    }

    #[test]
    fn circuit_independent_directions_respect_qoo() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = CircuitState::default();
        let decision = decide_circuit_sqm(
            1000,
            true,
            Some(90),
            &cpu,
            &circuits,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(90.0),
                up: Some(50.0),
            },
            &state,
        );
        assert_eq!(decision.down, Some(CircuitSqmState::FqCodel));
        assert_eq!(decision.up, None);
    }

    #[test]
    fn circuit_reverts_when_cpu_low() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = CircuitState::default();
        state.down = CircuitDirectionState {
            desired: CircuitSqmState::FqCodel,
            ..Default::default()
        };
        state.up = CircuitDirectionState {
            desired: CircuitSqmState::FqCodel,
            ..Default::default()
        };

        let decision = decide_circuit_sqm(
            1000,
            true,
            Some(10),
            &cpu,
            &circuits,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(90.0),
                up: Some(90.0),
            },
            &state,
        );
        assert_eq!(decision.down, Some(CircuitSqmState::Cake));
        assert_eq!(decision.up, Some(CircuitSqmState::Cake));
    }

    #[test]
    fn circuit_missing_rtt_blocks_downgrade() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let state = CircuitState::default();

        let decision = decide_circuit_sqm(
            1000,
            true,
            Some(90),
            &cpu,
            &circuits,
            &qoo_cfg,
            true,
            DownUpOrder {
                down: Some(90.0),
                up: Some(90.0),
            },
            &state,
        );
        assert_eq!(decision, CircuitSqmDecision::default());
    }

    #[test]
    fn circuit_dwell_time_blocks_switch() {
        let cpu = AutopilotCpuConfig::default();
        let circuits = AutopilotCircuitsConfig::default();
        let qoo_cfg = AutopilotQooConfig::default();
        let mut state = CircuitState::default();
        state.down.last_change_unix = Some(1000 - 60);
        state.up.last_change_unix = Some(1000 - 60);

        let decision = decide_circuit_sqm(
            1000,
            true,
            Some(90),
            &cpu,
            &circuits,
            &qoo_cfg,
            false,
            DownUpOrder {
                down: Some(90.0),
                up: Some(90.0),
            },
            &state,
        );
        assert_eq!(decision, CircuitSqmDecision::default());
    }

    #[test]
    fn directional_token_format_and_parse() {
        assert_eq!(
            format_directional_sqm_override(CircuitSqmState::Cake, CircuitSqmState::FqCodel),
            "cake/fq_codel"
        );

        let parsed = parse_directional_sqm_override("cake/fq_codel");
        assert_eq!(parsed.down, Some(CircuitSqmState::Cake));
        assert_eq!(parsed.up, Some(CircuitSqmState::FqCodel));

        let parsed = parse_directional_sqm_override("fq_codel");
        assert_eq!(parsed.down, Some(CircuitSqmState::FqCodel));
        assert_eq!(parsed.up, Some(CircuitSqmState::FqCodel));

        let parsed = parse_directional_sqm_override("none");
        assert_eq!(parsed.down, None);
        assert_eq!(parsed.up, None);

        let parsed = parse_directional_sqm_override("/fq_codel");
        assert_eq!(parsed.down, None);
        assert_eq!(parsed.up, Some(CircuitSqmState::FqCodel));
    }
}
