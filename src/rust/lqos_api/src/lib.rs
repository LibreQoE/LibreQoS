//! LibreQoS Local Node API (library)
//!
//! The API is not a service of its own: `lqosd` links this crate and mounts
//! [`api_routes`] at [`web::API_MOUNT_PREFIX`] on the node manager's listener.
//! There is no binary here, no listener of its own, and no licence gate — the
//! daemon's `x-bearer` verifier is the only authentication in front of these
//! routes.

pub mod api;
pub mod services;
pub mod transit_types;
pub mod web;

pub use api::*;
pub use web::{ApiDoc, api_routes};
