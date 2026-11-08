//! 74t168.b — TRIAL-DELIVERY CADENCE (lqosd control plane only).
//!
//! The trial→grant path FUNCTIONS but its delivery signal was PASSIVE: the
//! grant arrived SIX DAYS post-signup (Muhammed / bsz-qos-01) because nothing
//! on the box watched for the license actually becoming granted. This module
//! polls the server license status after a trial request (fixed 60 s, bounded
//! 24 h window — PRE-COMMITTED; the implementer may not change the cadence)
//! and reuses the existing signup poll-loop pattern — a second instance of an
//! existing pattern, NOT a new mechanism.
//!
//! The grant PUSH path (`license_grant::save_license_grant`) is UNTOUCHED:
//! polling OBSERVES; the push still delivers. The bridge picks the cap up via
//! its own mtime cache — no bridge change, no poke.
//!
//! Review edge (pinned, `docs/74t168ab_companions_review.md`): the status can
//! read GRANTED while the local grant has not yet landed (push in flight).
//! That LIMBO keeps polling within the window with a loud state-change log —
//! never terminal-granted on status alone (the grant must be present AND
//! verify locally).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use super::shared_types::LtsStatus;

/// PRE-COMMITTED cadence (driver-fixed): the server status endpoint is polled
/// once per minute (gentler than the signup loop's 10 s).
pub(crate) const TRIAL_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// PRE-COMMITTED bound (driver-fixed): the loop stops after 24 h regardless
/// (<= 1440 polls per request epoch).
pub(crate) const TRIAL_POLL_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// One request epoch's window start (unix seconds). A NEW trial request
/// resets it; the live loop reads it every tick, so the window follows the
/// latest request without a second loop.
static WINDOW_START_UNIX: AtomicU64 = AtomicU64::new(0);
/// One loop per request epoch.
static LOOP_LIVE: AtomicBool = AtomicBool::new(false);

/// The state after one observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrialDeliveryTick {
    /// Licensed status + the local grant is present and verifies => terminal.
    GrantedObserved,
    /// Not granted server-side yet — keep waiting.
    Waiting,
    /// Server says granted, but the local grant has NOT landed/verified
    /// (push in flight) — keep polling, log the limbo once.
    Limbo,
    /// The 24 h window elapsed without a grant => terminal WARN.
    WindowExpired,
}

/// The observation's disposition: what to log (state-change only) and whether
/// to stop the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrialDeliveryOutcome {
    pub tick: TrialDeliveryTick,
    /// Emit a log line (the tick differs from the last logged tick — the
    /// noise bar: one line per state change, zero periodic lines).
    pub log_state_change: bool,
    /// The loop ends after this observation.
    pub terminal: bool,
}

/// The pure, sequence-testable state machine of the delivery loop.
///
/// The async runner feeds observations (status, local-grant validity, elapsed
/// window time); this type owns the classification + the state-change logging
/// decision + the terminal signals.
#[derive(Debug)]
pub(crate) struct TrialDeliveryPoll {
    window: Duration,
    last_logged: Option<TrialDeliveryTick>,
    /// The local-grant tick-0 cold check already ran (first-tick rule).
    first_tick_done: bool,
}

impl TrialDeliveryPoll {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            window,
            last_logged: None,
            first_tick_done: false,
        }
    }

    /// Tick 0 (cold/first-tick rule): check the LOCAL grant BEFORE any
    /// network poll. An already-delivered grant terminates immediately with
    /// ZERO status fetches.
    pub(crate) fn first_tick(&mut self, local_grant_valid: bool) -> Option<TrialDeliveryOutcome> {
        if self.first_tick_done {
            return None;
        }
        self.first_tick_done = true;
        if local_grant_valid {
            self.last_logged = Some(TrialDeliveryTick::GrantedObserved);
            return Some(TrialDeliveryOutcome {
                tick: TrialDeliveryTick::GrantedObserved,
                log_state_change: true,
                terminal: true,
            });
        }
        None
    }

    /// Classify one observation and decide the disposition.
    ///
    /// Ordering: window expiry is terminal FIRST (a request whose 24 h
    /// elapsed stops even if the status would say granted — the caller warns
    /// and the operator re-requests); then the granted classes; the limbo
    /// (granted server-side, no local grant) is NOT terminal.
    pub(crate) fn observe(
        &mut self,
        status: LtsStatus,
        local_grant_valid: bool,
        elapsed: Duration,
    ) -> TrialDeliveryOutcome {
        let tick = if elapsed >= self.window {
            TrialDeliveryTick::WindowExpired
        } else if !status_is_granted(status) {
            TrialDeliveryTick::Waiting
        } else if local_grant_valid {
            TrialDeliveryTick::GrantedObserved
        } else {
            TrialDeliveryTick::Limbo
        };
        let log_state_change = self.last_logged != Some(tick);
        self.last_logged = Some(tick);
        TrialDeliveryOutcome {
            tick,
            log_state_change,
            terminal: matches!(
                tick,
                TrialDeliveryTick::GrantedObserved | TrialDeliveryTick::WindowExpired
            ),
        }
    }
}

/// The granted/licensed status classes. Consistent with the existing
/// `current_license_limits` predicate: anything but Invalid/NotChecked means
/// a grant is active server-side (FreeTrial -> Full as the trial converts).
pub(crate) fn status_is_granted(status: LtsStatus) -> bool {
    !matches!(status, LtsStatus::Invalid | LtsStatus::NotChecked)
}

/// The idempotence guard: ONE loop per request epoch. Returns true when the
/// caller should SPAWN (no live loop); false when a live loop exists — the
/// window start was still reset, so the existing loop follows the new
/// request (no double-spawn).
pub(crate) fn begin_trial_request_epoch(now_unix: u64) -> bool {
    WINDOW_START_UNIX.store(now_unix, Ordering::Release);
    LOOP_LIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// The loop's terminal hand-back (the runner calls this exactly once).
pub(crate) fn end_trial_delivery_loop() {
    LOOP_LIVE.store(false, Ordering::Release);
}

/// The elapsed time since the CURRENT window start (unix seconds).
pub(crate) fn window_elapsed(window_start_unix: u64, now_unix: u64) -> Duration {
    Duration::from_secs(now_unix.saturating_sub(window_start_unix))
}

pub(crate) fn current_window_start() -> u64 {
    WINDOW_START_UNIX.load(Ordering::Acquire)
}

/// Spawns the delivery poll for a trial request (cold path; the
/// `spawn_signup_poll_loop` pattern). Called on the request-success surface.
pub(crate) fn spawn_trial_delivery_poll() {
    let now = current_unix_seconds();
    if !begin_trial_request_epoch(now) {
        // A live loop exists; the window start was reset above and the loop
        // picks it up on its next tick (one loop per epoch, no double-spawn).
        return;
    }
    tokio::spawn(async move {
        let mut poll = TrialDeliveryPoll::new(TRIAL_POLL_WINDOW);
        // Tick 0: LOCAL-first (no network poll when the grant already landed).
        if let Some(outcome) = poll.first_tick(super::license_grant::current_valid_grant().is_some())
        {
            log_trial_delivery(outcome);
            end_trial_delivery_loop();
            return;
        }
        loop {
            let status = super::get_lts_license_status_async().await.0;
            let elapsed = window_elapsed(current_window_start(), current_unix_seconds());
            let outcome =
                poll.observe(status, super::license_grant::current_valid_grant().is_some(), elapsed);
            log_trial_delivery(outcome);
            if outcome.terminal {
                end_trial_delivery_loop();
                return;
            }
            tokio::time::sleep(TRIAL_POLL_INTERVAL).await;
        }
    });
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log_trial_delivery(outcome: TrialDeliveryOutcome) {
    if !outcome.log_state_change {
        return;
    }
    match outcome.tick {
        TrialDeliveryTick::GrantedObserved => {
            tracing::info!(
                "Insight trial DELIVERED: the grant is present and verifies locally; the \
                 bridge picks the licensed circuit cap up automatically (mtime cache)."
            );
        }
        TrialDeliveryTick::Limbo => {
            tracing::warn!(
                "Insight trial status is GRANTED server-side but the local grant has not \
                 landed yet (push in flight) — still polling within the 24h window."
            );
        }
        TrialDeliveryTick::WindowExpired => {
            tracing::warn!(
                "Insight trial delivery did NOT arrive within 24h of the request. The grant \
                 push appears stalled: re-request the trial or check the Insight account; \
                 the box stays on the fail-closed circuit cap until a grant verifies."
            );
        }
        TrialDeliveryTick::Waiting => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll() -> TrialDeliveryPoll {
        TrialDeliveryPoll::new(Duration::from_secs(24 * 60 * 60))
    }

    #[test]
    fn first_tick_already_granted_terminates_before_any_network_poll() {
        // §6.1(i): the cold/first-tick rule — a grant already on disk ends the
        // loop at tick 0 (the runner performs ZERO status fetches).
        let mut poll = poll();
        let outcome = poll
            .first_tick(true)
            .expect("an already-delivered grant must terminate at tick 0");
        assert_eq!(outcome.tick, TrialDeliveryTick::GrantedObserved);
        assert!(outcome.terminal);
        assert!(outcome.log_state_change, "the terminal is loud");
        // A second first_tick is a no-op (the tick already ran).
        assert!(poll.first_tick(true).is_none());
    }

    #[test]
    fn first_tick_not_granted_proceeds_to_polling() {
        let mut poll = poll();
        assert!(poll.first_tick(false).is_none(), "no local grant => poll");
    }

    #[test]
    fn granted_flip_on_poll_n_terminates_loudly() {
        // §6.1(ii): waiting -> granted on poll N: terminal + a state-change log.
        let mut poll = poll();
        poll.first_tick(false);
        let waiting = poll.observe(LtsStatus::NotChecked, false, Duration::from_secs(0));
        assert_eq!(waiting.tick, TrialDeliveryTick::Waiting);
        assert!(!waiting.terminal);
        // The FIRST observation is a state change (None -> Waiting); the
        // logger itself is silent for Waiting — the noise bar is the REPEAT
        // suppression asserted next.
        assert!(waiting.log_state_change);
        // Repeated waiting observations stay quiet (the noise bar).
        let quiet = poll.observe(LtsStatus::NotChecked, false, Duration::from_secs(60));
        assert!(!quiet.log_state_change, "zero periodic lines while waiting");
        // Server grants, the push lands: terminal.
        let granted = poll.observe(LtsStatus::FreeTrial, true, Duration::from_secs(120));
        assert_eq!(granted.tick, TrialDeliveryTick::GrantedObserved);
        assert!(granted.terminal);
        assert!(granted.log_state_change);
    }

    #[test]
    fn limbo_keeps_polling_and_never_terminates_on_status_alone() {
        // The pinned review edge: GRANTED server-side, local grant absent —
        // keep polling with a state-change log; never terminal on status only.
        let mut poll = poll();
        poll.first_tick(false);
        let limbo = poll.observe(LtsStatus::FreeTrial, false, Duration::from_secs(60));
        assert_eq!(limbo.tick, TrialDeliveryTick::Limbo);
        assert!(!limbo.terminal, "status alone is NOT terminal");
        assert!(limbo.log_state_change, "the limbo is surfaced once");
        let still = poll.observe(LtsStatus::FreeTrial, false, Duration::from_secs(120));
        assert!(!still.log_state_change, "one line per state change");
        // The push lands => terminal granted.
        let granted = poll.observe(LtsStatus::FreeTrial, true, Duration::from_secs(180));
        assert_eq!(granted.tick, TrialDeliveryTick::GrantedObserved);
        assert!(granted.terminal);
    }

    #[test]
    fn window_expiry_is_an_explicit_terminal() {
        // §6.1(iii): expiry => explicit terminal WARN, loop ends.
        let mut poll = TrialDeliveryPoll::new(Duration::from_secs(3_600));
        poll.first_tick(false);
        let expired = poll.observe(LtsStatus::NotChecked, false, Duration::from_secs(3_600));
        assert_eq!(expired.tick, TrialDeliveryTick::WindowExpired);
        assert!(expired.terminal);
        assert!(expired.log_state_change, "the expiry is loud");
    }

    #[test]
    fn transient_errors_retry_within_the_window() {
        // §6.1(iv): the status query's error path reads NotChecked (-1) — a
        // transient error is NOT terminal and never flips the terminal state.
        let mut poll = poll();
        poll.first_tick(false);
        for i in 0..5 {
            let outcome = poll.observe(
                LtsStatus::from_i32(-1),
                false,
                Duration::from_secs(60 * i),
            );
            assert_eq!(outcome.tick, TrialDeliveryTick::Waiting);
            assert!(!outcome.terminal, "transient => retry, never terminal");
        }
    }

    #[test]
    fn invalid_status_is_not_granted() {
        assert!(!status_is_granted(LtsStatus::Invalid));
        assert!(!status_is_granted(LtsStatus::NotChecked));
        assert!(status_is_granted(LtsStatus::FreeTrial));
        assert!(status_is_granted(LtsStatus::Full));
        assert!(status_is_granted(LtsStatus::SelfHosted));
    }

    #[test]
    fn one_loop_per_request_epoch_and_the_window_resets() {
        // §6.1(v): a second request while a loop is live must not double-spawn;
        // the window start still resets to the new request.
        end_trial_delivery_loop(); // normalize any prior test state
        assert!(begin_trial_request_epoch(1_000), "first request spawns");
        assert!(
            !begin_trial_request_epoch(2_000),
            "a live loop => no double-spawn"
        );
        assert_eq!(
            current_window_start(),
            2_000,
            "the new request resets the window for the live loop"
        );
        end_trial_delivery_loop();
        assert!(
            begin_trial_request_epoch(3_000),
            "after the loop ends a new request spawns"
        );
        end_trial_delivery_loop();
    }

    #[test]
    fn window_elapsed_math() {
        assert_eq!(
            window_elapsed(1_000, 1_060),
            Duration::from_secs(60),
            "seconds since the window start"
        );
        assert_eq!(window_elapsed(1_000, 900), Duration::ZERO, "monotone (no future underflow)");
    }
}
