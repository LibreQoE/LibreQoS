//! Public types for reading, writing, and merging LibreQoS override files.
#![warn(missing_docs)]

mod file_lock;
mod overrides_file;
mod topology_overrides;
pub use overrides_file::{
    CircuitAdjustment, NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore,
    TopologyParentOverrideMode, UispOverrides, UispRouteOverride,
};
pub use topology_overrides::{
    AttachmentProbePolicy, ManualAttachment, ManualAttachmentGroup, TopologyAttachmentMode,
    TopologyAttachmentOverride, TopologyOverridesFile,
};
