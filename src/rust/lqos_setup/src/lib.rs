//! Shared setup support for the `lqos_setup` binary.

pub mod bootstrap;
pub mod config_migrate;
pub mod hotfix;
pub mod nic_bind;
pub mod port_class;
pub mod power;
pub mod ssl;

pub use bootstrap::PostinstAction;
