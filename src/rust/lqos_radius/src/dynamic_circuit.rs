//! Dynamic-circuit command intents emitted by RADIUS session handling.

use crate::{AccountingEvent, AccountingSessionKey, NasResetStatus};

/// Sink boundary for dynamic-circuit intents produced by RADIUS session state.
pub trait DynamicCircuitCommandSink {
    /// Receives one dynamic-circuit intent.
    ///
    /// Side effects: depend on the sink implementation. The `lqos_radius` crate
    /// only calls this boundary; it does not write dynamic circuit files or talk
    /// to `lqosd` directly.
    fn emit(&mut self, intent: DynamicCircuitIntent);
}

/// Dynamic-circuit intent that an lqosd-facing adapter can map onto daemon commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DynamicCircuitIntent {
    /// Create a runtime dynamic circuit. Adapters can map this to `CreateDynamicCircuit`.
    CreateDynamicCircuit(DynamicCircuitUpsert),
    /// Update a runtime dynamic circuit. Adapters can map this to `CreateDynamicCircuit`.
    UpdateDynamicCircuit(DynamicCircuitUpsert),
    /// Remove a runtime dynamic circuit. Adapters can map this to `RemoveDynamicCircuit`.
    RemoveDynamicCircuit(DynamicCircuitRemoval),
}

impl DynamicCircuitIntent {
    /// Returns the stable dynamic-circuit identifier carried by this intent.
    #[must_use]
    pub fn circuit_id(&self) -> &str {
        match self {
            Self::CreateDynamicCircuit(upsert) | Self::UpdateDynamicCircuit(upsert) => {
                &upsert.circuit_id
            }
            Self::RemoveDynamicCircuit(removal) => &removal.circuit_id,
        }
    }
}

/// Data needed to create or update a dynamic circuit from a RADIUS session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicCircuitUpsert {
    /// Stable circuit identifier for the dynamic circuit overlay.
    pub circuit_id: String,
    /// Deterministic RADIUS session key that produced this intent.
    pub session_key: AccountingSessionKey,
    /// Latest decoded accounting event data for the shapeable session.
    pub event: AccountingEvent,
}

/// Data needed to remove a dynamic circuit from a RADIUS session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicCircuitRemoval {
    /// Stable circuit identifier for the dynamic circuit overlay.
    pub circuit_id: String,
    /// Deterministic RADIUS session key that produced this intent.
    pub session_key: AccountingSessionKey,
    /// Why this removal was emitted.
    pub reason: DynamicCircuitRemovalReason,
}

/// Cause for a dynamic-circuit removal intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicCircuitRemovalReason {
    /// The RADIUS session emitted Acct-Status-Type Stop.
    Stop,
    /// The in-memory session was expired by the caller.
    Expired,
    /// A previously shapeable session no longer has enough data to stay active.
    NoLongerShapeable,
    /// A session was promoted to a different deterministic dynamic-circuit id.
    Rekeyed,
    /// A NAS reset event marked the session stale.
    NasReset(NasResetStatus),
}
