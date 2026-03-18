---
name: libreqos-python-workflow
description: Shared LibreQoS workflow for Python orchestration, scheduler, integrations, and helper scripts under src/. Use when changing LibreQoS.py, scheduler.py, integration*.py, integrationCommon.py, Python helpers, or Python tests around shaping and generated config files.
---

# LibreQoS Python Workflow

Use this skill for Python work under `src/`.

## Scope

- Main Python shaper entrypoint: `src/LibreQoS.py`
- Scheduler/orchestration: `src/scheduler.py`
- Shared importer graph/output logic: `src/integrationCommon.py`
- Importers and helper scripts: `src/integration*.py`, `src/lqTools.py`, `src/configMigrator.py`, `src/csvToNetworkJSON.py`, `src/cidrToHosts.py`, `src/mikrotikFindIPv6.py`
- Python tests: `src/test_*.py`, `src/bakery_integration_test.py`, `src/testGraph.py`

## Architecture

- `LibreQoS.py` is the live shaper/orchestrator. It validates inputs, builds queueing state, coordinates TC/XDP work through `liblqos_python`, and writes runtime artifacts such as `queuingStructure.json`, `statsByCircuit.json`, and `statsByParentNode.json`.
- `scheduler.py` runs integrations, applies overrides, updates scheduler status for the Web UI, and triggers full or partial refreshes.
- `integrationCommon.py` is the shared graph/output layer for integrations and writes `network.json` and `ShapedDevices.csv`.
- Python in this repo is operational glue around `liblqos_python` and installed runtime files, not an isolated app package.

## Invariants

- Do not require a venv. System-Python compatibility is intentional.
- Treat `ShapedDevices.csv` and `network.json` as shared contracts across integrations, scheduler, overrides, and shaping.
- Preserve stable circuit and device identities emitted by integrations unless the change explicitly includes a migration plan for overrides and downstream consumers.
- Preserve tolerant input handling for operator-managed files: BOMs, UTF-16/non-UTF8 CSVs, comment stripping, and uneven rows are all present in current workflows for a reason.
- Preserve scheduler resilience: importer failures should be logged/reported and should not kill the scheduler loop.
- Preserve existing external protocols and identity values for integrations unless the task explicitly includes coordinated changes on both sides.
- Be careful with paths: some code uses `get_libreqos_directory()`, while some helpers and tests intentionally use cwd-relative files.

## Validation

- Run targeted tests from `src/`, not repo root, for example:
  - `python3 -m unittest test_scheduler.py`
  - `python3 -m unittest test_shaping_skip_report.py`
  - `python3 -m unittest test_virtual_tree_nodes.py`
- If changing scheduler/helpers, prefer the smallest focused unittest set that covers the touched logic.
- If changing generated file behavior, review both the writer and the reader side in the same change.

## References

- Read `references/generated-runtime-files.md` when deciding whether a file is source input, generated output, or runtime state.
- Read `references/integration-modes.md` when touching scheduler/importer selection, stable identities, or integration output assumptions.

## High-Risk Changes

- Anything that changes `ShapedDevices.csv` columns, row semantics, or encoding behavior
- Anything that changes `network.json` structure or virtual-node handling
- Scheduler subprocess/error-handling behavior
- Runtime path handling or current-working-directory assumptions
- Live shaping entrypoints that can touch TC/XDP state
- Broad "cleanup" refactors that try to package-ize or normalize the Python tree

## Notes

- `ispConfig.py` is useful as a historical/example config reference, but current runtime config is largely surfaced through `liblqos_python` and `/etc/lqos.conf`.
- Historical files such as `LibreQoS-old.py`, `LibreQoS-ancient.py`, and `LibreQoS.py.new` exist in-tree but are not the default source of truth.
- If a Python change adds or newly requires shipped files, update `src/build_dpkg.sh` in the same change.
