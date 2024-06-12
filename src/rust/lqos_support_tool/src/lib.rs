//! Provides a support library for the support tool system.
mod support_info;
pub mod console;
mod sanity_checks;

pub use support_info::gather_all_support_info;
pub use support_info::SupportDump;
pub use sanity_checks::run_sanity_checks;