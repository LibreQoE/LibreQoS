//! `[maintenance_window]` — the control-plane publish gate (bead `lqos-8h8`).
//!
//! When the broker's `step()` is about to emit a generation whose shape is in
//! the remaining PAUSE set AND the configured site-local window is closed, the
//! broker HOLDS the desired artifact (never emits) and re-evaluates each tick.
//! At window-open (or a per-change apply-now / a global break-glass / the
//! `force_apply_hours` bound) it emits; `semantic_diff(desired, applied)`
//! coalesces the net change into ONE generation. The ENGINE is untouched.
//!
//! Card: `docs/lqos_maintenance_window_card.md` (§6 config + §13/§14 defaults).
//! Decisions (Q1..Q12 adopted): predicted class table; ONE window,
//! midnight-crossing allowed; a SINGLE GLOBAL site TZ; `chrono-tz` for DST;
//! a malformed/zero-length window ⇒ defer NOTHING + WARN; force from the
//! OLDEST deferred; gate the PUBLISH; the content-key identity; a time-boxed
//! global audited break-glass; the predicate covers EVERY door; cancel = revert
//! the desired; gate ONLY pause-class publishes.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}
fn default_start_local() -> String {
    "03:00".to_string()
}
fn default_end_local() -> String {
    "04:00".to_string()
}
fn default_max_pause_ms() -> u64 {
    25
}
fn default_force_apply_hours() -> u64 {
    24
}

/// The `[maintenance_window]` section.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct MaintenanceWindowConfig {
    /// Decision 2: the window is ON by default. `false` ⇒ the gate is inert
    /// (window always open) — a single flip restores today's behaviour
    /// byte-for-byte (§12 flags).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Site-local window start (`"HH:MM"`).
    #[serde(default = "default_start_local")]
    pub start_local: String,
    /// Site-local window end (`"HH:MM"`). `end <= start` ⇒ the window CROSSES
    /// midnight (Q2).
    #[serde(default = "default_end_local")]
    pub end_local: String,
    /// Decision 3 / Q3: ONE global IANA timezone (e.g. `"America/Denver"`).
    /// `""` ⇒ the server-local zone (`/etc/localtime`). Q4: DST-correct via a
    /// TZ database.
    #[serde(default)]
    pub timezone: String,
    /// Decision 1: only shapes whose PREDICTED pause exceeds this (ms) are
    /// gated. **RESERVED — a NO-OP today**: the static (predicted) class table
    /// treats EVERY pause class as over the bound, so this knob changes nothing
    /// (a reviewer nit). It is the calibration bound the Q1
    /// *measured* refinement will filter on once a per-class predicted pause (ms)
    /// exists. Resolved + logged at broker start (`MAINTENANCE_WINDOW_RESOLVED`).
    #[serde(default = "default_max_pause_ms")]
    pub max_pause_ms: u64,
    /// Decision 6 / Q6: a deferred change older than this is FORCED (emitted
    /// regardless of the window; from the OLDEST). Bounds staleness.
    #[serde(default = "default_force_apply_hours")]
    pub force_apply_hours: u64,
}

impl Default for MaintenanceWindowConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            start_local: default_start_local(),
            end_local: default_end_local(),
            timezone: String::new(),
            max_pause_ms: default_max_pause_ms(),
            force_apply_hours: default_force_apply_hours(),
        }
    }
}

/// Parses an `"HH:MM"` (24-hour) string to minutes-since-midnight. `None` when
/// unparseable or out of range.
pub fn parse_local_hhmm(value: &str) -> Option<u32> {
    let (h, m) = value.trim().split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

impl MaintenanceWindowConfig {
    /// The parsed window in minutes-since-midnight, or `None` when disabled or
    /// malformed / zero-length (Decision 7 / Q5: a malformed window defers
    /// NOTHING — the caller treats `None` as "always open"). Loud once at the
    /// call site, not here.
    pub fn window_minutes(&self) -> Option<(u32, u32)> {
        if !self.enabled {
            return None;
        }
        let start = parse_local_hhmm(&self.start_local)?;
        let end = parse_local_hhmm(&self.end_local)?;
        if start == end {
            return None; // zero-length ⇒ defer nothing
        }
        Some((start, end))
    }

    /// `true` when `now_minutes` falls inside the window. Handles the
    /// midnight-crossing case (`end <= start` ⇒ `[start, 1440) ∪ [0, end)`).
    pub fn is_open_at(&self, now_minutes: u32) -> bool {
        match self.window_minutes() {
            None => true, // disabled / malformed / zero-length ⇒ always open
            Some((start, end)) if end > start => now_minutes >= start && now_minutes < end,
            Some((start, end)) => now_minutes >= start || now_minutes < end,
        }
    }

    /// §6 / Q5: validate only a STRUCTURALLY unparseable value (a loud load
    /// error). A zero-length window is NOT an error — the runtime normalises it
    /// to always-open (Decision 7).
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled {
            if parse_local_hhmm(&self.start_local).is_none() {
                return Err(format!(
                    "maintenance_window.start_local must be \"HH:MM\" (got {:?})",
                    self.start_local
                ));
            }
            if parse_local_hhmm(&self.end_local).is_none() {
                return Err(format!(
                    "maintenance_window.end_local must be \"HH:MM\" (got {:?})",
                    self.end_local
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(start: &str, end: &str) -> MaintenanceWindowConfig {
        MaintenanceWindowConfig {
            start_local: start.to_string(),
            end_local: end.to_string(),
            ..MaintenanceWindowConfig::default()
        }
    }

    #[test]
    fn simple_window_is_open_inside_only() {
        let c = cfg("03:00", "04:00");
        assert!(!c.is_open_at(2 * 60));
        assert!(c.is_open_at(3 * 60));
        assert!(c.is_open_at(3 * 60 + 59));
        assert!(!c.is_open_at(4 * 60));
        assert!(!c.is_open_at(23 * 60));
    }

    #[test]
    fn midnight_crossing_window_wraps() {
        let c = cfg("23:00", "01:00");
        assert!(c.is_open_at(23 * 60 + 30));
        assert!(c.is_open_at(0));
        assert!(c.is_open_at(30));
        assert!(!c.is_open_at(60));
        assert!(!c.is_open_at(12 * 60));
    }

    #[test]
    fn disabled_and_malformed_are_always_open() {
        let mut c = cfg("03:00", "04:00");
        c.enabled = false;
        assert!(c.is_open_at(12 * 60));
        let bad = cfg("nope", "04:00");
        assert!(bad.is_open_at(12 * 60)); // malformed ⇒ defer nothing
        let zero = cfg("03:00", "03:00");
        assert!(zero.is_open_at(3 * 60)); // zero-length ⇒ defer nothing
    }

    #[test]
    fn validate_rejects_structurally_unparseable() {
        assert!(cfg("03:00", "04:00").validate().is_ok());
        assert!(cfg("25:00", "04:00").validate().is_err());
        assert!(cfg("03:00", "99:99").validate().is_err());
        let mut off = cfg("bad", "worse");
        off.enabled = false; // disabled ⇒ not validated
        assert!(off.validate().is_ok());
    }
}
