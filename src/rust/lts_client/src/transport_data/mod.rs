//! Holds data-types and utility functions for the long-term
//! statistics retention system.
//!
//! This is in the bus so that it can be readily shared between
//! server and client code.

mod license_types;
mod license_utils;
mod submissions;

pub use license_types::*;
pub use license_utils::*;
pub use submissions::*;

pub(crate) const LICENSE_SERVER: &str = "license.libreqos.io:9126";
