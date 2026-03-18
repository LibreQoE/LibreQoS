# Generated And Runtime Files

Use this reference when deciding whether a file is source-of-truth input, generated output, or runtime state.

## Source Inputs

- `ShapedDevices.csv`: operator- or integration-managed shaping input consumed by `LibreQoS.py`
- `network.json`: operator- or integration-managed topology input consumed by `LibreQoS.py`
- `network.insight.json` / `ShapedDevices.insight.csv`: Insight-topology variants when enabled

## Integration Writers

- `integrationCommon.py` writes `network.json` and `ShapedDevices.csv`
- `scheduler.py` may rewrite `ShapedDevices.csv` and `network.json` when applying overrides

## Shaper Outputs

- `queuingStructure.json`: runtime queue/tree snapshot written by `LibreQoS.py`
- `statsByCircuit.json`, `statsByParentNode.json`: runtime stats snapshots written by `LibreQoS.py`
- `linux_tc.txt` and related `linux_tc*.txt` files: runtime/debug outputs from shaping work
- `lastRun.txt`: last successful shaper run marker
- `ShapedDevices.lastLoaded.csv`: snapshot of the last loaded shaping input
- `lastGoodConfig.csv`, `lastGoodConfig.json`: fallback copies of last validated good inputs
- `planner_state.json`: runtime planner state used by shaping logic

## Agent Guidance

- Prefer editing source inputs and code, not generated/runtime outputs.
- When changing a writer, review every reader of that file in the same change.
- Preserve stable file names and semantics unless the task explicitly includes a coordinated migration.
