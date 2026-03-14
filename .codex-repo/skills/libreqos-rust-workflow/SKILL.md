---
name: libreqos-rust-workflow
description: Shared LibreQoS Rust workflow for repo contributors. Use when changing Rust under src/rust, validating Rust crates, deciding between workspace commands and --manifest-path, or applying LibreQoS-specific Rust conventions and verification steps.
---

# LibreQoS Rust Workflow

Use this skill for Rust work in this repo.

## Scope

- Rust sources live under `src/rust/`.
- `src/rust/Cargo.toml` is the source of truth for current workspace members.
- Some crates exist in-tree but are not current workspace members. If a crate is outside `[workspace].members`, use `cargo --manifest-path path/to/Cargo.toml`.

## Workflow

1. Read `AGENTS.md` first for current repo rules and crate descriptions.
2. Identify whether the touched crate is a workspace member.
3. Validate the touched crate with `cargo check -p <crate>` when possible.
4. Run relevant tests.
5. Run `cargo clippy` for the touched crate and fix actionable issues.
6. If dependencies changed, also run:
   - `cargo machete`
   - `cargo audit`
   - `cargo tree`
7. If the change adds, renames, moves, or newly depends on runtime files, static assets, helper scripts, service files, templates, or install-time artifacts, review and update `src/build_dpkg.sh` in the same change.

## Preferred Rust Direction

- Prefer `parking_lot` for new `Mutex` and `RwLock` usage.
- Prefer `crossbeam_channel` for new MPSC/MPMC channels.
- Prefer `thiserror` for structured errors.
- Prefer `let else` and early returns over deeply nested `if let`.
- Avoid introducing new `pub static` values with locks when helper functions or actors are better.
- Avoid introducing new `#[inline(always)]`; prefer `#[inline]`.
- Keep RustDoc current for changed public items and note side effects for non-pure functions.
- Avoid allocation in hot paths.

## Notes

- Existing code does not fully match all preferred conventions yet. Treat these as direction for new and touched code, not as a reason to perform unrelated cleanup.
- Build/package scripts live under `src/`, not repo root.
- `src/build_dpkg.sh` is a functional packaging manifest for shipped installs. Forgetting to update it is a common failure mode; treat package-content drift as a bug.
- For Insight/LTS2 integrations, preserve the existing external protocol and identity values unless the task explicitly covers coordinated changes on both sides. Unilateral protocol drift is a breaking change.
