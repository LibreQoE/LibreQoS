//! Shared topology compilation layer for projecting imported integration topology
//! into operator-selectable topology modes.

#![warn(missing_docs)]

mod bundle;
mod compile_mode;
mod validation;

pub use bundle::{
    CompiledTopologyBundle, ImportedTopologyBundle, TopologyCompiledShapingFile, TopologyImportFile,
};
pub use compile_mode::{TopologyCompileError, TopologyCompileMode, compile_topology};
pub use validation::validate_compiled_bundle;
