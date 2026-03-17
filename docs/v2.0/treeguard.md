# TreeGuard

TreeGuard is a current LibreQoS v2.0 feature for intelligent node management.

Important status:

1. TreeGuard is **enabled by default** in LibreQoS v2.0.
2. TreeGuard can manage both eligible node virtualization and per-circuit SQM policy.
3. Operators can tune or disable TreeGuard in `/etc/lqos.conf` or the WebUI TreeGuard page.

## What TreeGuard Does

TreeGuard has two control domains:

1. Link/node virtualization management (for selected nodes).
2. Per-circuit SQM switching between `cake` and `fq_codel`.

For circuits, TreeGuard can make per-direction decisions (download and upload independently).

## Default Behavior in LibreQoS v2.0

In LibreQoS v2.0, TreeGuard is enabled by default.

By default, TreeGuard may virtualize enrolled nodes and may switch enrolled circuit directions between `cake diffserv4` and `fq_codel` according to its configured guardrails.

If you prefer fixed/manual behavior, disable TreeGuard or narrow its enrollment lists.

## Circuit SQM Switching Model

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

Current default behavior:

```toml
[treeguard]
enabled = true
dry_run = false
tick_seconds = 1

[treeguard.links]
enabled = true
all_nodes = true
top_level_auto_virtualize = true

[treeguard.circuits]
enabled = true
all_circuits = true
switching_enabled = true
independent_directions = true

[treeguard.qoo]
enabled = true
```

## Safe Rollout Pattern

1. Review TreeGuard settings early in deployment instead of assuming static/manual queue behavior.
2. If you want a narrower rollout, disable `all_nodes` and/or `all_circuits` and use allowlists first.
3. Validate behavior over multiple peak/off-peak windows.
4. If you want observation-only validation, set `dry_run = true` temporarily.
5. If you need fixed/manual behavior, set `enabled = false`.

## Overrides and Operational Notes

When enabled and not dry-run, TreeGuard may persist decisions to:

- `lqos_overrides.treeguard.json`

TreeGuard is designed to avoid fighting operator-owned overrides. If operator overrides exist for enrolled entities, TreeGuard skips those entities and reports warnings.

## Related Pages

- [HTB + fq_codel + CAKE: Detailed Queueing Behavior](htb_fq_codel_cake.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [LibreQoS Backend Architecture](libreqos-backend-architecture.md)
- [Future Development Inputs](future-development-inputs.md)
