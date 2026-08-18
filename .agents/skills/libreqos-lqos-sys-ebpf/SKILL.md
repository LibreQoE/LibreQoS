---
name: libreqos-lqos-sys-ebpf
description: Shared LibreQoS workflow for lqos_sys eBPF, C wrapper, bindgen, and pinned-map ABI work. Use when changing src/rust/lqos_sys/build.rs, src/rust/lqos_sys/src/bpf/**, wrapper.c or wrapper.h, lqos_sys Rust mirror structs, or XDP/TC attachment and map ABI behavior.
---

# LibreQoS lqos_sys eBPF Workflow

Use this skill for `lqos_sys` BPF/FFI work.

## Scope

- Source of truth lives under `src/rust/lqos_sys/`.
- eBPF/C code lives under `src/rust/lqos_sys/src/bpf/`.
- Rust includes generated bindings from `OUT_DIR`; do not edit generated artifacts directly.

## Build Pipeline

`build.rs` orchestrates this pipeline:

1. Compile `src/bpf/lqos_kern.c` to LLVM IR with `clang`
2. Convert LLVM IR to a BPF object with `llc`
3. Generate `lqos_kern_skel.h` with `bpftool gen skeleton`
4. Compile `src/bpf/wrapper.c`
5. Archive the wrapper into a small static library
6. Run `bindgen` on `wrapper.h`
7. Include generated bindings from `OUT_DIR/bindings.rs`

When changing any part of that chain, treat the whole chain as one linked surface.

## High-Risk Changes

- BPF map key/value structs
- Shared constants such as sizing limits
- `wrapper.h` / `wrapper.c` function signatures
- Skeleton-exposed program or map names
- Pinned-map ABI expectations in `lqos_kernel.rs`

## Required Checklist

1. Edit only source files under `src/rust/lqos_sys/`.
2. If a BPF-side struct changes, update the Rust mirror type in the same change.
3. Update or preserve size assertions for Rust mirror types.
4. Review pinned-map ABI cleanup in `src/rust/lqos_sys/src/lqos_kernel.rs`.
5. Run `cargo check -p lqos_sys`.
6. Run relevant tests for touched Rust mirror types and map readers.
7. If runtime attach/detach behavior changed, state clearly whether live root validation was performed.
8. After any repo change, invoke `heckler` via `$libreqos-review-subagents-workflow` before returning to the user.
9. After each source-code implementation batch, also invoke `reaper` via `$libreqos-review-subagents-workflow`.

## Mirror Types To Check

- `src/rust/lqos_sys/src/throughput.rs`
- `src/rust/lqos_sys/src/flowbee_data.rs`
- `src/rust/lqos_sys/src/ip_mapping/ip_hash_data.rs`
- `src/rust/lqos_sys/src/ip_mapping/ip_hash_key.rs`

## Safety Rules

- Do not edit `OUT_DIR` files.
- Do not assume a Rust-only change is safe if it reads a BPF map with a mirrored struct layout.
- Do not run live XDP/TC attach-detach or pinned-map cleanup without explicit user approval; this can disrupt networking on the host.
- Be careful with verifier-sensitive patterns in BPF code: bounds checks, scratch maps for large structs, and hot-path allocation avoidance are intentional.

## Common Failure Modes

- `clang`, `llc`, or `bpftool` missing from the host
- Skeleton generation succeeding but wrapper/bindgen drifting
- Rust mirror struct sizes no longer matching BPF-side layouts
- Stale pinned maps silently reusing an incompatible ABI

## Notes

- `build.rs` treats stderr from `clang` and `llc` as fatal.
- Pinned map compatibility cleanup already exists for several maps in `lqos_kernel.rs`; extend it when introducing new ABI-sensitive pinned maps.
