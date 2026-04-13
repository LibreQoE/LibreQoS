---
name: libreqos-packaging-release
description: Shared LibreQoS workflow for packaging, release, and shipped-file changes. Use when changing src/build_dpkg.sh, src/build_rust.sh, src/rust/lqosd/copy_files.sh, packaged binaries/assets/templates/service files, or any file that must exist on installed LibreQoS systems.
---

# LibreQoS Packaging and Release

Use this skill for packaging and shipped-file work.

## Scope

- Packaging scripts: `src/build_dpkg.sh`, `src/build_rust.sh`
- Node manager asset copy/build path: `src/rust/lqosd/copy_files.sh`, `src/rust/lqosd/dev_build.sh`
- Files copied into installed layouts such as `/opt/libreqos/src`, `/opt/libreqos/src/bin`, `/etc/systemd/system`, and packaged static assets

## Invariants

- `src/build_dpkg.sh` is the functional source of truth for shipped `.deb` installs.
- `src/build_rust.sh` and `src/build_dpkg.sh` should stay aligned where they overlap on binaries, static assets, helper scripts, and operator expectations.
- If a change adds, renames, moves, or newly requires a shipped file, update packaging in the same change.
- Never assume packaging will pick up a new file automatically. Verify the copy/install path explicitly.

## Workflow

1. Read `AGENTS.md` first for current packaging and safety rules.
2. Identify every new or changed file the feature needs at runtime, install time, or service start time.
3. Review `src/build_dpkg.sh` and any related copy/build helpers for the relevant copy/install path.
4. Update the packaging/build scripts in the same change as the feature.
5. If node_manager assets are involved, also review `copy_files.sh`, `dev_build.sh`, and the node_manager build contract.
6. If install behavior or service expectations changed, update docs in the same change.

## References

- Read `references/package-surface.md` when deciding whether a file is part of the shipped install or where it is copied from.

## Validation

- Run `bash -n` on changed shell scripts.
- If node_manager assets are involved, run `src/rust/lqosd/src/node_manager/js_build/test-build-contract.sh`.
- During large sessions, invoke `$libreqos-review-subagents-workflow` (and `helen` if UI changed).
- If you did not run a full package build, say so explicitly.
- Before finishing, answer this directly: "Does `src/build_dpkg.sh` include every new required file?"

## High-Risk Changes

- Service files and post-install behavior
- Static assets copied through intermediate build helpers
- Python/Rust files that work in-repo but are missing from the installed package
- `update_api.sh` / `lqos_api` packaging behavior
- Any change that assumes `/opt/libreqos/src` contains files that packaging does not copy

## Notes

- Package-content drift is a recurring failure mode in this repo.
- `src/build_rust.sh` can restart services on the local host; treat it as an operationally sensitive script, not a harmless build helper.
