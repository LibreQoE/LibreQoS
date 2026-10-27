//! Managed netplan inspection and transaction helper for LibreQoS.

#![deny(clippy::unwrap_used)]

pub mod inspect;
pub mod protocol;
pub mod transaction;

pub use inspect::{
    DetectedNetplanFile, NetplanEnslavement, NetworkModeInspection, bind_guard, bind_guard_at,
    dataplane_netplan_enslavement, dataplane_netplan_enslavement_at, inspect_network_mode,
};
pub use protocol::{
    ApplyMode, ApplyRequest, ApplyResponse, BackupSummary, HelperStatus, PendingOperationStatus,
};
