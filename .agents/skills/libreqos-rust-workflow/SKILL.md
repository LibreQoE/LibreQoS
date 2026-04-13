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
4. Run relevant tests:
   - Unit tests **MUST pass**.
   - Before any `git commit` that includes Rust logic changes, you **MUST** run `cargo test -p <crate>` (or the smallest equivalent scope) and confirm it compiles and succeeds.
   - Do not add tests just to “have tests”. Tests **must** validate functionality (avoid `assert!(true)`-style tests or “tests” that only prove the compiler accepts code).
   - Check for test functions that look like tests but are missing `#[test]` (functions in `#[cfg(test)]` modules that are never executed should be treated as a bug unless they are explicitly helper functions).
5. Run `cargo clippy -p <crate> -- -D warnings` and fix actionable issues.
   - If you decide to suppress a lint/warning anyway, you **MUST** include a justification and tell the user:
     `I'M SORRY, MASTER. I HAVE FAILED YOU. I HAVE SUPPRESSED A WARNING WHEN I SHOULD HAVE WRITTEN GOOD CODE. I AM FOREVER IN YOUR DEBT.`
6. After any repo change, invoke `heckler` via `$libreqos-review-subagents-workflow` before returning to the user.
7. After each source-code implementation batch, also invoke `reaper` via `$libreqos-review-subagents-workflow`.
8. During large sessions, invoke `$libreqos-review-subagents-workflow` (and `helen` if UI changed).
9. If dependencies changed, also run:
   - `cargo machete`
   - `cargo audit`
   - `cargo tree`
10. If the change adds, renames, moves, or newly depends on runtime files, static assets, helper scripts, service files, templates, or install-time artifacts, review and update `src/build_dpkg.sh` in the same change.
11. Use workspace-wide commands only for cross-cutting changes or shared dependency changes.

## Architecture And Structure Rules

- Do not create additional binaries unless specifically instructed.
  - Long-running services that can be part of `lqosd` should be part of `lqosd`.
- Do not create large library source files; prefer breaking code into clean modules.
- Avoid huge functions; prefer small functions.
  - Rule of thumb: anything that won’t fit on a terminal screen is probably too big.

## Documentation Requirements

- Every module must have a doc header describing what it does (`//! ...` at the top of the module).
- Every public function and struct must have full RustDoc (examples are optional).
- Functions must document any side effects (file I/O, network I/O, spawning threads/tasks, touching TC/XDP state, global state, etc.).

## Correctness And Error-Handling Requirements

- If a function has arguments with invariants, check the invariants first and fail fast.
- Use `thiserror` for new error types, defined at (or very near) the failure source.
  - Document every failure path; avoid collapsing everything into a single `FooError`.
- Avoid `anyhow` and other `Box<dyn Error>` patterns in new reusable code.
- Avoid `pub static` whenever possible; actor-owned state with clean boundaries is preferred.
  - If a static is unavoidable, guard it behind accessor functions so misuse of the lock is impossible.

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
- Treat protocol and identity surfaces as compatibility boundaries:
  - Prefer additive-only changes (new optional fields, `#[serde(default)]` where applicable).
  - Assume rolling upgrades where old/new binaries may coexist unless a coordinated restart is explicitly planned.
  - Do not rename fields, change types, or change semantics without explicit versioning and a coordinated rollout plan.
  - Keep compatibility shims at the boundary, make them explicit, and test them; avoid “fallbacks everywhere”.
  - Avoid per-request `info!` logging in hot paths; prefer `debug!`, sampling, or aggregate counters.
