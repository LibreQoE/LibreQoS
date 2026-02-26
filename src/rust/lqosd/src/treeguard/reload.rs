//! Topology reload helpers for TreeGuard.
//!
//! Link virtualization changes require a scheduler reload to take effect. This module provides a
//! simple cooldown + exponential backoff controller to avoid flapping reloads.

use lqos_config::load_libreqos;

const MIN_COOLDOWN_SECONDS: u64 = 60;
const MAX_BACKOFF_SECONDS: u64 = 2 * 60 * 60; // 2 hours

/// Result of a reload attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    /// Reload succeeded.
    Success {
        /// Output from the reload command.
        message: String,
    },
    /// Reload was skipped because cooldown/backoff is active.
    Skipped {
        /// Human-readable reason for skipping.
        reason: String,
        /// Next UNIX timestamp when a reload may be attempted.
        next_allowed_unix: Option<u64>,
    },
    /// Reload failed (and backoff has been applied).
    Failed {
        /// Human-readable error.
        error: String,
        /// Next UNIX timestamp when a reload may be attempted.
        next_allowed_unix: Option<u64>,
    },
}

/// Cooldown/backoff controller for topology reload requests.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReloadController {
    last_attempt_unix: Option<u64>,
    consecutive_failures: u32,
    backoff_until_unix: Option<u64>,
}

impl ReloadController {
    /// Attempts to reload LibreQoS, applying cooldown and exponential backoff.
    ///
    /// This function is not pure: it shells out via `lqos_config::load_libreqos()`.
    pub(crate) fn try_reload(&mut self, now_unix: u64, cooldown_minutes: u64) -> ReloadOutcome {
        let cooldown_seconds = cooldown_minutes
            .saturating_mul(60)
            .max(MIN_COOLDOWN_SECONDS);

        if let Some(until) = self.backoff_until_unix {
            if now_unix < until {
                return ReloadOutcome::Skipped {
                    reason: "Reload backoff active".to_string(),
                    next_allowed_unix: Some(until),
                };
            }
        }

        if let Some(last) = self.last_attempt_unix {
            let next = last.saturating_add(cooldown_seconds);
            if now_unix < next {
                return ReloadOutcome::Skipped {
                    reason: "Reload cooldown active".to_string(),
                    next_allowed_unix: Some(next),
                };
            }
        }

        self.last_attempt_unix = Some(now_unix);

        match load_libreqos() {
            Ok(message) => {
                self.consecutive_failures = 0;
                self.backoff_until_unix = None;
                ReloadOutcome::Success { message }
            }
            Err(e) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let backoff_seconds = backoff_seconds(cooldown_seconds, self.consecutive_failures);
                let next = now_unix.saturating_add(backoff_seconds);
                self.backoff_until_unix = Some(next);
                ReloadOutcome::Failed {
                    error: e.to_string(),
                    next_allowed_unix: Some(next),
                }
            }
        }
    }
}

/// Computes the backoff window in seconds given a cooldown and consecutive failure count.
///
/// This function is pure: it has no side effects.
fn backoff_seconds(cooldown_seconds: u64, consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return cooldown_seconds;
    }

    let exp = consecutive_failures.saturating_sub(1).min(10); // cap exponent
    let mult = 1u64 << exp;
    cooldown_seconds
        .saturating_mul(mult)
        .min(MAX_BACKOFF_SECONDS)
}
