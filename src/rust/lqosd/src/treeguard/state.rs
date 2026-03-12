//! TreeGuard per-entity state.
//!
//! This module will hold per-node and per-circuit state such as dwell timers,
//! last-seen timestamps, and smoothed telemetry.

use std::collections::VecDeque;

/// Smoothed state using an exponential weighted moving average (EWMA).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ewma {
    initialized: bool,
    value: f64,
}

impl Ewma {
    /// Updates the EWMA with a new sample and returns the updated value.
    ///
    /// This function is not pure: it mutates `self`.
    pub fn update(&mut self, sample: f64, alpha: f64) -> f64 {
        if !self.initialized {
            self.initialized = true;
            self.value = sample;
            return self.value;
        }

        self.value = (alpha * sample) + ((1.0 - alpha) * self.value);
        self.value
    }

    /// Returns the current EWMA value if it has been initialized.
    ///
    /// This function is pure: it has no side effects.
    pub fn current(&self) -> Option<f64> {
        if self.initialized {
            Some(self.value)
        } else {
            None
        }
    }
}

/// Virtualization state for a managed link/node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkVirtualState {
    /// Node should be present in the physical shaping tree.
    #[default]
    Physical,
    /// Node should be marked virtual (logical-only).
    Virtual,
}

/// SQM profile state for a managed circuit direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CircuitSqmState {
    /// Use CAKE (higher CPU cost, higher quality).
    #[default]
    Cake,
    /// Use fq_codel (lower CPU cost).
    FqCodel,
}

/// Per-direction link tracking state.
#[derive(Clone, Debug, Default)]
pub struct LinkDirectionState {
    /// Smoothed utilization percentage.
    pub util_ewma_pct: Ewma,
    /// When the link first went idle (seconds since UNIX epoch), if currently idle.
    pub idle_since_unix: Option<u64>,
    /// When the link first went below the configured top-level safe-util threshold (seconds since
    /// UNIX epoch), if currently below.
    pub top_level_safe_since_unix: Option<u64>,
}

/// Per-node link virtualization tracking state.
#[derive(Clone, Debug, Default)]
pub struct LinkState {
    /// Current desired virtualization state as managed by TreeGuard.
    pub desired: LinkVirtualState,
    /// Last state change time (seconds since UNIX epoch), if any.
    pub last_change_unix: Option<u64>,
    /// History of recent state changes (seconds since UNIX epoch), newest at the back.
    pub recent_changes_unix: VecDeque<u64>,
    /// Download direction tracking state.
    pub down: LinkDirectionState,
    /// Upload direction tracking state.
    pub up: LinkDirectionState,
}

/// Per-direction circuit SQM switching tracking state.
#[derive(Clone, Debug, Default)]
pub struct CircuitDirectionState {
    /// Current desired SQM profile for this direction.
    pub desired: CircuitSqmState,
    /// Last state change time (seconds since UNIX epoch), if any.
    pub last_change_unix: Option<u64>,
    /// History of recent SQM switches (seconds since UNIX epoch), newest at the back.
    pub recent_changes_unix: VecDeque<u64>,
    /// Smoothed utilization percentage.
    pub util_ewma_pct: Ewma,
    /// When the circuit direction first went idle (seconds since UNIX epoch), if currently idle.
    pub idle_since_unix: Option<u64>,
}

/// Per-circuit SQM switching tracking state.
#[derive(Clone, Debug, Default)]
pub struct CircuitState {
    /// Download direction SQM state.
    pub down: CircuitDirectionState,
    /// Upload direction SQM state.
    pub up: CircuitDirectionState,
}

/// Returns true if both directions have been idle for at least `idle_min_minutes`.
///
/// This function is pure: it has no side effects.
pub fn is_sustained_window(
    now_unix: u64,
    down_since_unix: Option<u64>,
    up_since_unix: Option<u64>,
    min_minutes: u32,
) -> bool {
    let (Some(down_since), Some(up_since)) = (down_since_unix, up_since_unix) else {
        return false;
    };

    let min_secs = u64::from(min_minutes).saturating_mul(60);
    now_unix.saturating_sub(down_since) >= min_secs && now_unix.saturating_sub(up_since) >= min_secs
}

pub fn is_sustained_idle(
    now_unix: u64,
    down_idle_since_unix: Option<u64>,
    up_idle_since_unix: Option<u64>,
    idle_min_minutes: u32,
) -> bool {
    is_sustained_window(
        now_unix,
        down_idle_since_unix,
        up_idle_since_unix,
        idle_min_minutes,
    )
}

#[cfg(test)]
mod tests {
    use super::is_sustained_idle;

    #[test]
    fn sustained_idle_requires_both_directions() {
        assert!(!is_sustained_idle(1000, Some(0), None, 15));
        assert!(!is_sustained_idle(1000, None, Some(0), 15));
        assert!(!is_sustained_idle(1000, None, None, 15));
    }

    #[test]
    fn sustained_idle_requires_full_duration() {
        let now = 1_000_000u64;
        // 14:59 is not enough for 15 minutes
        assert!(!is_sustained_idle(
            now,
            Some(now - 899),
            Some(now - 899),
            15
        ));
        // Exactly 15 minutes is enough
        assert!(is_sustained_idle(now, Some(now - 900), Some(now - 900), 15));
    }
}
