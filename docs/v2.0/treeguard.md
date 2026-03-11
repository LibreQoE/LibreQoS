# TreeGuard (Upcoming v2.0 Feature)

TreeGuard is an upcoming LibreQoS v2.0 feature for intelligent node management.

Important status:

1. TreeGuard is **upcoming**.
2. TreeGuard is **not enabled by default**.
3. Current defaults remain unchanged unless an operator explicitly enables TreeGuard.

## What TreeGuard Does

TreeGuard has two control domains:

1. Link/node virtualization management (for selected nodes).
2. Per-circuit SQM switching between `cake` and `fq_codel`.

For circuits, TreeGuard can make per-direction decisions (download and upload independently).

## Default Behavior Without TreeGuard

When TreeGuard is not enabled, LibreQoS behavior stays with configured/default SQM policy (commonly `cake diffserv4`, with operator overrides where configured).

TreeGuard does not affect traffic unless enabled.

## Circuit SQM Switching Model (When Enabled)

TreeGuard evaluates utilization, RTT freshness, CPU guardrails, and optional QoO guardrails.

High-level behavior:

1. For sustained low-load conditions, TreeGuard may switch a direction from `cake` to `fq_codel`.
2. If utilization rises, RTT/QoO guardrails are unsafe, or revert conditions are met, TreeGuard switches back to `cake`.
3. Decisions can be independent per direction when `independent_directions = true`.

This gives a dynamic profile where loaded directions favor `cake diffserv4`, while low-load directions can use `fq_codel` when conditions are safe.

## Configuration (`/etc/lqos.conf`)

TreeGuard config lives under `[treeguard]` and sub-sections:

1. `[treeguard]`: enable/disable, dry-run, tick cadence.
2. `[treeguard.cpu]`: CPU-aware vs traffic/RTT mode and thresholds.
3. `[treeguard.links]`: node virtualization enrollment and guardrails.
4. `[treeguard.circuits]`: circuit enrollment and SQM switching guardrails.
5. `[treeguard.qoo]`: optional QoO protection threshold.

From PR #946 defaults:

```toml
[treeguard]
enabled = false
dry_run = true
tick_seconds = 1

[treeguard.circuits]
enabled = true
switching_enabled = true
independent_directions = true
idle_util_pct = 2.0
idle_min_minutes = 15
rtt_missing_seconds = 120
upgrade_util_pct = 5.0
min_switch_dwell_minutes = 30
max_switches_per_hour = 4
persist_sqm_overrides = true
```

## Safe Rollout Pattern

1. Keep `enabled = false` until you have reviewed policy and enrollment lists.
2. Start with `enabled = true` and `dry_run = true`.
3. Use a small allowlist of nodes/circuits first.
4. Validate behavior over multiple peak/off-peak windows.
5. Only then set `dry_run = false`.

## Overrides and Operational Notes

When enabled and not dry-run, TreeGuard may persist decisions to:

- `lqos_overrides.treeguard.json`

TreeGuard is designed to avoid fighting operator-owned overrides. If operator overrides exist for enrolled entities, TreeGuard skips those entities and reports warnings.

## Related Pages

- [HTB + fq_codel + CAKE: Detailed Queueing Behavior](htb_fq_codel_cake.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [LibreQoS Backend Architecture](libreqos-backend-architecture.md)
- [Future Development Inputs](future-development-inputs.md)
