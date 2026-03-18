# Integration Modes

Use this reference when working on scheduler/importer behavior.

## Scheduler Selection

- `scheduler.importFromCRM()` uses an `elif` chain for automatic integrations.
- In normal automatic mode, only the first enabled integration runs.
- This is not a fan-out scheduler for running multiple automatic importers in one pass.

## UISP Special Case

- Automatic UISP import is handled through the compiled `bin/uisp_integration` binary.
- `integrationUISP.py` still exists in-tree, but the scheduler's automatic UISP path does not call it directly.

## Shared Contracts

- Integrations normalize external systems into `network.json` and `ShapedDevices.csv`.
- Stable circuit IDs and device IDs are important for overrides, planner state, and partial reload behavior.
- Preserve existing external protocols and identity values unless the task explicitly includes coordinated work on both sides.

## Agent Guidance

- Do not assume all integrations are equally mature; error handling and path behavior vary across modules.
- When changing integration output shape or identity rules, review scheduler overrides and `LibreQoS.py` consumers in the same change.
