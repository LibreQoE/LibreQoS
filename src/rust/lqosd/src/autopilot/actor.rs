//! Autopilot actor loop.
//!
//! The actor is responsible for sampling telemetry, maintaining state machines,
//! and applying (or dry-running) any decisions.

use crate::autopilot::errors::AutopilotError;

/// Starts the Autopilot actor.
///
/// This function has side effects: it spawns background work.
pub(crate) fn start_autopilot_actor() -> Result<(), AutopilotError> {
    Ok(())
}

