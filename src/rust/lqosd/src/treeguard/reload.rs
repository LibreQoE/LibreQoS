//! Topology reload helpers for TreeGuard.
//!
//! Link virtualization changes require a scheduler reload to take effect. This module provides a
//! simple cooldown + exponential backoff controller to avoid flapping reloads.

use crate::reload_lock::{try_reload_libreqos_locked, ReloadExecOutcome};

const MIN_COOLDOWN_SECONDS: u64 = 60;
const MAX_BACKOFF_SECONDS: u64 = 2 * 60 * 60; // 2 hours

/// Reload request priority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReloadPriority {
    /// Normal reload requests obey cooldown/backoff.
    #[default]
    Normal,
    /// Urgent reload requests bypass cooldown (but still respect backoff).
    Urgent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkipKind {
    Backoff,
    Cooldown,
    Busy,
}

/// Result of polling the reload controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReloadAttempt {
    pub(crate) outcome: ReloadOutcome,
    pub(crate) request_reason: Option<String>,
}

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
    pending: bool,
    pending_priority: ReloadPriority,
    pending_reason: Option<String>,
    last_reported_skip: Option<(SkipKind, Option<u64>)>,
}

impl ReloadController {
    /// Enqueues a reload request with a given priority and human-readable reason.
    ///
    /// This function is not pure: it mutates internal pending state.
    pub(crate) fn request_reload(&mut self, priority: ReloadPriority, reason: String) {
        if !self.pending {
            self.pending = true;
            self.pending_priority = priority;
            self.pending_reason = Some(reason);
        } else {
            if priority == ReloadPriority::Urgent {
                self.pending_priority = ReloadPriority::Urgent;
            }
            match self.pending_reason.as_deref() {
                None => self.pending_reason = Some(reason),
                Some(existing) => {
                    if existing != reason && existing != "Multiple node topology changes" {
                        self.pending_reason = Some("Multiple node topology changes".to_string());
                    }
                }
            }
        }
        // Ensure we emit at least one skip outcome for a newly requested reload.
        self.last_reported_skip = None;
    }

    /// Polls for a pending reload request and attempts it if allowed.
    ///
    /// Returns `Some(...)` when an outcome should be surfaced to the activity log; repeated
    /// skip outcomes are suppressed to avoid flooding the activity ring.
    pub(crate) fn poll_reload(
        &mut self,
        now_unix: u64,
        cooldown_minutes: u32,
    ) -> Option<ReloadAttempt> {
        if !self.pending {
            return None;
        }

        let cooldown_seconds = u64::from(cooldown_minutes)
            .saturating_mul(60)
            .max(MIN_COOLDOWN_SECONDS);

        if let Some(until) = self.backoff_until_unix {
            if now_unix < until {
                return self.maybe_report_skip(
                    SkipKind::Backoff,
                    Some(until),
                    "Reload backoff active".to_string(),
                    Some(until),
                );
            }
        }

        if self.pending_priority == ReloadPriority::Normal {
            if let Some(last) = self.last_attempt_unix {
                let next = last.saturating_add(cooldown_seconds);
                if now_unix < next {
                    return self.maybe_report_skip(
                        SkipKind::Cooldown,
                        Some(next),
                        "Reload cooldown active".to_string(),
                        Some(next),
                    );
                }
            }
        }

        match try_reload_libreqos_locked() {
            ReloadExecOutcome::Busy => self.maybe_report_skip(
                SkipKind::Busy,
                None,
                "Reload already in progress".to_string(),
                None,
            ),
            ReloadExecOutcome::Success(message) => {
                self.last_attempt_unix = Some(now_unix);
                self.consecutive_failures = 0;
                self.backoff_until_unix = None;
                self.pending = false;
                self.pending_priority = ReloadPriority::Normal;
                let why = self
                    .pending_reason
                    .take()
                    .unwrap_or_else(|| "Topology change".to_string());
                self.last_reported_skip = None;
                Some(ReloadAttempt {
                    outcome: ReloadOutcome::Success { message },
                    request_reason: Some(why),
                })
            }
            ReloadExecOutcome::Failed(error) => {
                self.last_attempt_unix = Some(now_unix);
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let backoff_seconds = backoff_seconds(cooldown_seconds, self.consecutive_failures);
                let next = now_unix.saturating_add(backoff_seconds);
                self.backoff_until_unix = Some(next);
                self.last_reported_skip = None;
                let why = self
                    .pending_reason
                    .clone()
                    .unwrap_or_else(|| "Topology change".to_string());
                Some(ReloadAttempt {
                    outcome: ReloadOutcome::Failed {
                        error,
                        next_allowed_unix: Some(next),
                    },
                    request_reason: Some(why),
                })
            }
        }
    }

    fn maybe_report_skip(
        &mut self,
        kind: SkipKind,
        key_next_allowed_unix: Option<u64>,
        reason: String,
        next_allowed_unix: Option<u64>,
    ) -> Option<ReloadAttempt> {
        let key = (kind, key_next_allowed_unix);
        if self.last_reported_skip == Some(key) {
            return None;
        }
        self.last_reported_skip = Some(key);
        Some(ReloadAttempt {
            outcome: ReloadOutcome::Skipped {
                reason,
                next_allowed_unix,
            },
            request_reason: None,
        })
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
