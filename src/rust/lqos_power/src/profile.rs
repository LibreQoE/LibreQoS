//! Re-export of the config-owned [`PowerProfile`] so the power core, the
//! installer, and the CLI all share ONE type (the `[power] profile` single
//! source).

pub use lqos_config::PowerProfile;
