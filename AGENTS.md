LibreQoS is a Quality-of-Experience traffic management and analysis system. It's open source (GPL2). It is supported
by a paid Insight package (aka LTS2) and a paid API (lqos_api).

# Layout

src/                        Main source tree: Python entrypoints, packaging/build scripts, templates, test data, generated/runtime files
src/rust/                   Rust source tree and workspace root
docs/                       English documentation
docs-es/                    Spanish documentation
docker/                     Container-related files and configs
sim/                        Simulation/support assets

## Python

Most Python entrypoints live in `src/`, not the repo root.

src/LibreQoS.py             Main shaper/orchestrator. Validates inputs, builds the shaping tree, programs TC/XDP state, and writes runtime stats/artifacts.
src/scheduler.py            Background scheduler. Runs integrations, applies overrides, updates Web UI scheduler status, and triggers refreshes.
src/integrationCommon.py    Shared integration graph/model layer. Builds `network.json` and `ShapedDevices.csv` from importer data.
src/integration*.py         CRM/NMS importers and related helpers. UISP may run via Rust binary; several others still run in Python.
src/virtual_tree_nodes.py   Logical-to-physical network helpers for virtual nodes in `network.json`.
src/shaping_skip_report.py  Helper for reporting devices/circuits that were not attached during shaping.
src/lqTools.py              Operator/debug helper CLI for inspecting active shaping state.
src/configMigrator.py       Compatibility helper for migrating old `ispConfig.py` settings into newer config flows.
src/csvToNetworkJSON.py     Helper script for generating `network.json` from CSV input.
src/cidrToHosts.py          Helper script for expanding CIDRs in `ShapedDevices.csv`.
src/mikrotikFindIPv6.py     Optional Mikrotik helper used by some integrations for IPv6 enrichment.
src/pythonCheck.py          Shared Python version gate used by executable Python entrypoints.
src/test_*.py               Lightweight unit tests. These generally assume the working directory is `src/`.
src/bakery_integration_test.py  Larger integration-oriented test harness for shaping/tree behavior.
src/LibreQoS-old.py         Historical copy; not the primary source of truth for current work.
src/LibreQoS-ancient.py     Historical copy; not the primary source of truth for current work.
src/LibreQoS.py.new         Staging/alternate copy; not the primary source of truth unless the task explicitly targets it.
src/VERSION_STRING          Current release string, also used by packaging/build scripts.

## Rust

`src/rust/Cargo.toml` is the source of truth for current workspace members.

### Current workspace members

lqosd                       Main daemon. Also contains the node manager / web UI server.
xdp_iphash_to_cpu_cmdline   Compatibility CLI for managing IP mappings in the XDP cpumap system.
xdp_pping                   Legacy compatibility CLI for the earlier `xdp_pping` workflow.
uisp_integration            Rust UISP integration binary.
uisp                        Shared UISP API/types used by the integration.
lqusers                     CLI for managing LibreQoS web logins.
lqtop                       CLI/TUI for displaying live network status.
lqos_utils                  Shared helpers and utility types.
lqos_sys                    XDP/TC/eBPF bridge and kernel-program management.
lqos_stormguard             Experimental dynamic rate-adjustment system driven by live link-quality/capacity data.
lqos_setup                  First-run/setup CLI.
lqos_python                 PyO3 bindings exposing Rust config/bus/helpers to Python.
lqos_overrides              Override library/CLI for persistent devices, circuit adjustments, network adjustments, and UISP overrides.
lqos_map_perf               eBPF/XDP map performance CLI.
lqos_heimdall               Heimdall packet/flow watchlist and capture support.
lqos_config                 Configuration management, including config, shaped devices, and network.json handling.
lqos_bus                    Local bus types and Unix-domain-socket support used to communicate with lqosd.
lqos_bakery                 Queue creation/update/delete and diff-driven queue management.

### Other Rust crates/directories present in-tree

lqos_queue_tracker          Reads and tracks Linux `tc` queue state for the rest of the system.
lqos_support_tool           Support/sanity-check CLI for gathering and submitting support dumps.
third_party/libbpf-sys      Vendored libbpf-sys tree used by the Rust side.

## Helper Scripts

src/build_rust.sh           Builds the Rust side in release mode, installs binaries/artifacts into `src/bin`, updates `liblqos_python.so`, and may restart services
src/build_dpkg.sh           Builds LibreQoS and assembles a `.deb` package
src/rust/lqosd/dev_build.sh Builds node_manager JS bundles and copies static assets into `src/bin/static2` for local UI iteration
src/lqosd/src/node_manager/js_build/esbuild.sh  Builds all JavaScript and puts the web system in the right place (must run from that directory)
src/rust/lqosd/src/node_manager/js_build/test-build-contract.sh  Verifies the node_manager page/build/vendor contract before bundling
.codex-repo/link-skills.sh  Symlinks repo-owned shared Codex skills into `~/.codex/skills`

## Shared Codex Skills

Canonical shared Codex skills for this repo live in `.codex-repo/skills/`.

- These skills are intended to be committed to the repo and shared across developers.
- Developers should link them into `~/.codex/skills` with `.codex-repo/link-skills.sh`.
- After linking new skills, or pulling updates that change them, restart Codex so the skill list refreshes.
- When a LibreQoS repo skill is installed and the task matches it, use it.
- If your changes touch `.codex-repo/skills/` or `.codex-repo/link-skills.sh`, remind the user to run `./.codex-repo/link-skills.sh` and restart Codex so updated shared skills are picked up.

# Code Rules

## Generated Artifacts

- Treat built outputs, generated files, and runtime artifacts as derived state unless the task explicitly targets the generator or a test fixture.
- Do not hand-edit generated/built outputs such as `src/bin/**`, `dist/**`, `target/**`, `src/rust/lqosd/src/node_manager/js_build/out/**`, generated `OUT_DIR` artifacts, packaged `.deb` trees, runtime graphs/images/logs, or shaping snapshots like `queuingStructure.json`, `statsByCircuit.json`, `statsByParentNode.json`, `lastRun.txt`, and `linux_tc*.txt`.
- Edit the source inputs, rebuild/regenerate, and then review the result.

## Operational Safety

- Any command that can affect live shaping, TC/XDP state, network interfaces, system services, package install/uninstall state, or installed runtime files under `/opt/libreqos` or `/etc` requires explicit user approval.
- Prefer non-destructive validation first: targeted tests, `cargo check`, shell syntax checks, build-contract checks, and static review before any live-host or service-affecting action.

## Rust

- For Rust changes, prefer validating touched workspace members with `cargo check -p package`, `cargo test`, and `cargo clippy`.
- Some crates present under `src/rust/` are not current workspace members. For those, use `cargo --manifest-path path/to/Cargo.toml`.
- Use `cargo machete` when changing Rust dependencies in the workspace.
- Use `cargo audit`. Failures with no mitigation, unmaintained crates are acceptable.
- Use `cargo tree` when adding dependencies and try to minimize the number of dependencies with different versions in the tree.
- Prefer `parking_lot` for new Mutex/RwLock usage. Existing code still contains some `std::sync` locks and atomics.
- Prefer `crossbeam_channel` for new MPSC/MPMC channels. Existing code still contains some `std::sync::mpsc`.
- Public Rust functions and types should have RustDoc. Update RustDoc when changing behavior.
- Avoid nested `if let` where `let else` with early return is clearer.
- Prefer `thiserror` defined errors. Log the error during `map_err` while converting to the error type.
    - Avoid error messages that lack specificity. "File not found" is not ok; "File (filename) not found" is great.
- In most cases, prefer an actor (thread or async) over a fine-grained lock.
- Avoid introducing new `pub static` values with locks when helper functions or actors would work better. Existing code still has some shared globals.
- Prefer pure functions. If a pure function can be constified, do it.
- If a function is NOT pure, the RustDoc should document any side effects.
- Avoid introducing new `#[inline(always)]`; prefer `#[inline]`. Existing hot-path code may still contain `#[inline(always)]`.
- Consider inline markers for functions in the hot path, such as XDP ring-buffer/perf-buffer processing.
- Avoid allocation in the hot path.

## Python

- If a task touches `src/LibreQoS.py`, `src/scheduler.py`, `src/integration*.py`, `src/integrationCommon.py`, or Python helpers/tests around shaping and integrations, use the `libreqos-python-workflow` repo skill if installed.
- Do not add a mandatory venv/poetry/pipenv workflow. This repo intentionally supports system-Python installs for operational compatibility.
- Treat `ShapedDevices.csv` and `network.json` as functional contracts shared between integrations, overrides, the scheduler, and `LibreQoS.py`. Do not casually rename them, relocate them, or change their schema/semantics.
- Preserve stable circuit/device identity values emitted by integrations unless the task explicitly calls for a coordinated migration. Overrides, partial reload logic, and downstream tooling depend on those identifiers staying stable.
- Preserve tolerant file handling for operator-managed inputs. Existing code intentionally accepts BOMs, UTF-16/non-UTF8 CSVs, comment lines, and uneven row shapes in places like `ShapedDevices.csv`.
- Preserve scheduler fault tolerance. Integration failures should surface to scheduler output/error reporting and continue scheduling, not terminate the scheduler process.
- Preserve current path behavior carefully. Some Python code uses `get_libreqos_directory()` for installed runtime paths, while other helpers/tests intentionally read and write files relative to the current working directory.
- Prefer `get_libreqos_directory()` when adding new installed-runtime path lookups, but do not "normalize" existing cwd-based scripts without reviewing all callers, packaging, and operator workflows.
- Keep `pythonCheck.checkPythonVersion()` at the top of executable Python entrypoints unless the script is being intentionally retired.
- Prefer focused `python3 -m unittest ...` runs from `src/` for touched Python tests. Existing tests are not laid out as a package and often assume that working directory.
- Prefer incremental hardening over large Python rewrites. This code is operational glue with many install-time and user-data assumptions; avoid broad refactors unless the task explicitly calls for them.
- Do not run `src/LibreQoS.py` or other live-shaping Python entrypoints against the host without explicit user approval; they can modify active TC/XDP state and scheduler-visible files.

### lqos_sys BPF/FFI

- If a task touches `src/rust/lqos_sys/build.rs`, `src/rust/lqos_sys/src/bpf/**`, `src/rust/lqos_sys/src/bpf/wrapper.c`, `src/rust/lqos_sys/src/bpf/wrapper.h`, or Rust structs mirroring eBPF map layouts, use the `libreqos-lqos-sys-ebpf` repo skill if installed.
- Never edit generated `OUT_DIR` artifacts. Edit only source files under `src/rust/lqos_sys/`.
- Treat BPF-side structs, Rust mirror structs, bindgen output expectations, and pinned-map ABI handling as one linked surface.
- When changing BPF map key/value structs or shared constants, update the Rust mirror types and size assertions in the same change.
- When changing pinned-map ABI, review the map compatibility cleanup in `src/rust/lqos_sys/src/lqos_kernel.rs`.
- Do not run live XDP/TC attach-detach or pinned-map cleanup without explicit user approval. These actions can disrupt host networking.

### Node Manager Frontend

- If a task touches `src/rust/lqosd/src/node_manager/js_build/**`, `src/rust/lqosd/src/node_manager/static2/**`, `src/rust/lqosd/copy_files.sh`, `src/rust/lqosd/dev_build.sh`, or `src/rust/lqosd/src/node_manager/static_pages.rs`, use the `libreqos-node-manager-frontend` repo skill if installed.
- Treat `template.html`, `node_manager.css`, `static2/vendor/`, `js_build/entrypoints.txt`, `js_build/esbuild.sh`, `js_build/test-build-contract.sh`, `copy_files.sh`, `dev_build.sh`, and `static_pages.rs` as one linked surface.
- Node manager frontend dependencies are Bootstrap 5, FontAwesome, vendored assets, and page-specific bundled JS. Do not infer frontend dependencies from `src/package.json`.
- Preserve the existing node_manager look and feel. Use `src/rust/lqosd/src/node_manager/static2/template.html`, `src/rust/lqosd/src/node_manager/static2/node_manager.css`, and the existing pages as the baseline rather than introducing a new design system or framework.
- Prefer vendored frontend assets under `src/rust/lqosd/src/node_manager/static2/vendor/`. Avoid new CDN or npm dependencies unless explicitly required.
- When adding or renaming a node_manager page, update `js_build/src/<page>.js`, `js_build/entrypoints.txt`, `static2/<page>.html`, and `static_pages.rs` together.
- Run `src/rust/lqosd/src/node_manager/js_build/test-build-contract.sh` after changes to the node_manager frontend build path.

### Insight Integration Compatibility

- Any communication with Insight/LTS2 must preserve the existing external protocol, message shapes, field names, identity values, and compatibility expectations by default.
- Do not change Insight-facing protocol details or identity semantics unless the task explicitly includes coordinated work on both LibreQoS and the Insight side.
- If a task appears to require an Insight protocol or identity change but the paired Insight-side update is not in scope, stop and surface that as a blocker instead of making a unilateral compatibility break.
- Treat Insight protocol drift as a breaking change, even if the local LibreQoS side compiles or tests cleanly.

## Scripts

- If a task touches `src/build_rust.sh`, `src/build_dpkg.sh`, `src/rust/lqosd/copy_files.sh`, shipped service files, packaged templates/assets, or anything that must exist on installed LibreQoS systems, use the `libreqos-packaging-release` repo skill if installed.
- Any change to `src/build_rust.sh` or `src/build_dpkg.sh` MUST be reflected in the other where applicable. They should stay in sync on shared build/package assumptions.
- `src/build_dpkg.sh` is part of the functional source of truth for shipped LibreQoS installs. If a change adds, renames, moves, or newly requires files at runtime, packaging time, or install time, update `src/build_dpkg.sh` in the same change.
- Never assume a new file is "obvious enough" to be picked up automatically by packaging. Verify that `src/build_dpkg.sh` explicitly includes every new file or directory required for the feature to work in an installed `.deb`.
- When a change affects packaged assets, treat "is `src/build_dpkg.sh` still accurate?" as a mandatory review check before finishing.

## Documentation

- Always keep the documentation updated to match code changes.
- Public docs in `docs/` and `docs-es/` are ISP/operator-facing by default. Write them for customers using LibreQoS, not for contributors, Codex, or internal support staff.
- Do not write public docs like internal change logs, AGENTS notes, implementation plans, or support-only commentary.
- Prefer operator-visible behavior, setup steps, and validation guidance over browser/server mechanics, internal file-flow details, or code-architecture commentary.
- If a topic is mainly for contributors, support, or engineering, keep it in clearly separate technical/reference documentation rather than mixing it into customer pages.
- Keep Spanish public docs fully translated and idiomatic; avoid leaking English implementation terms unless they are the actual UI label.
- Follow `DOCS_STYLE.md` when editing public documentation.
