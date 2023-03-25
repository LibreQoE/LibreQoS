//! Ports of C code that is very Linux specific.

mod possible_cpus;
mod txq_base_setup;
pub use possible_cpus::num_possible_cpus;
pub(crate) use txq_base_setup::*;