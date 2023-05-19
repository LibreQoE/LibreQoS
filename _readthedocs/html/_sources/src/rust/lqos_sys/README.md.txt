## lqos_sys

This crate wraps the XDP component in externally callable Rust. This is
used by other systems to manage the XDP/TC eBPF system.

The `src/bpf` directory contains the C for the eBPF program, as well as
some wrapper helpers to bring it into Rust-space.
