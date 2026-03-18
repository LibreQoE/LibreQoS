//! Public types for reading, writing, and merging LibreQoS override files.
#![warn(missing_docs)]

mod overrides_file;
pub use overrides_file::{
    CircuitAdjustment, NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore,
    UispOverrides, UispRouteOverride,
};
