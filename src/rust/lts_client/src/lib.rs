//! Shared data and functionality for the long-term statistics system.

/// Transport data and helpers for the long-term statistics system.
pub mod transport_data;

/// Shared public key infrastructure data and functionality for the long-term statistics system.
pub mod pki;

/// Collection system for `lqosd`
pub mod collector;

/// Submissions system for `lqosd`
pub mod submission_queue;
pub use collector::CakeStats;

/// Re-export bincode
pub mod bincode {
  pub use bincode::*;
}

/// Re-export CBOR
pub mod cbor {
    pub use serde_cbor::*;
}

/// Re-export dryocbox
pub mod dryoc {
  pub use dryoc::*;
}