//! Autopilot subsystem for intelligent node management.
//!
//! This module provides an actor-driven control loop that can:
//! - monitor link utilization and RTT,
//! - virtualize/unvirtualize selected network nodes, and
//! - adjust per-circuit shaping behavior to reduce CPU load.

pub(crate) mod actor;
pub(crate) mod bakery;
pub(crate) mod decisions;
pub(crate) mod errors;
pub(crate) mod overrides;
pub(crate) mod reload;
pub(crate) mod state;
pub(crate) mod status;

pub(crate) use errors::AutopilotError;

