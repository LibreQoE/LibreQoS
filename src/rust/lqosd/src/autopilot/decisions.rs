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
        AutopilotCpuMode::ManualProfiles => false,
    }
}

/// Returns true if CPU headroom calls for reverting CPU-saving actions.
///
/// This function is pure: it has no side effects.
fn cpu_calls_for_revert(cpu: &AutopilotCpuConfig, cpu_max_pct: Option<u8>) -> bool {
    match cpu.mode {
        AutopilotCpuMode::CpuAware => cpu_max_pct.is_some_and(|pct| pct <= cpu.cpu_low_pct),
        AutopilotCpuMode::TrafficRttOnly => false,
        AutopilotCpuMode::ManualProfiles => false,
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
    cpu_max_pct: Option<u8>,
    cpu_cfg: &AutopilotCpuConfig,
    circuits_cfg: &AutopilotCircuitsConfig,
    qoo_cfg: &AutopilotQooConfig,
    rtt_missing: bool,
    qoo: DownUpOrder<Option<f32>>,
    state: &CircuitState,
) -> CircuitSqmDecision {
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
