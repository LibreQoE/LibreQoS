//! Shared setup support for the `lqos_setup` binary.

pub mod bootstrap;
pub mod hotfix;
pub mod ssl;

pub use bootstrap::PostinstAction;
