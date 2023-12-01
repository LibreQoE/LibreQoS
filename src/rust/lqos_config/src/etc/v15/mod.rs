//! Handles the 1.5.0 configuration file format.

mod top_config;
pub use top_config::Config;
mod anonymous_stats;
mod tuning;
mod bridge;
mod long_term_stats;