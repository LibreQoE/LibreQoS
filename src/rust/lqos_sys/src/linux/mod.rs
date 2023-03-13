//! Ports of C code that is very Linux specific.

mod possible_cpus;
pub use possible_cpus::num_possible_cpus;