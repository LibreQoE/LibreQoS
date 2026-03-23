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
2. If utilization rises, QoO guardrails are unsafe, or revert conditions are met, TreeGuard switches back toward the circuit's base SQM policy.
3. Decisions can be independent per direction when `independent_directions = true`.

This gives a dynamic profile where loaded directions favor `cake diffserv4`, while low-load directions can use `fq_codel` when conditions are safe.

The base SQM policy comes from operator intent, not from TreeGuard defaults. In practice, TreeGuard starts from the effective configured policy for each circuit and only persists its own temporary overlay when it needs to differ from that base.

Important base-policy rule:

1. If a direction's base SQM policy is `cake`, TreeGuard may temporarily switch that direction to `fq_codel` and later return it to base.
2. If a direction's base SQM policy is `fq_codel`, TreeGuard does not circuit-switch that direction to `cake`.
3. Link virtualization remains available regardless of the circuit SQM base policy.

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

[treeguard.cpu]
mode = "cpu_aware"
cpu_high_pct = 75
cpu_low_pct = 55

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

TreeGuard node virtualization is intended to be CPU-aware by default. Traffic, RTT, and QoO
remain important safety and restore signals, but new automatic virtualization should happen when
CPU pressure suggests HTB savings are worthwhile. Upgraded installs from older defaults are
silently migrated from `traffic_rtt_only` to `cpu_aware`, with a visible notice in logs/UI.

## Safe Rollout Pattern

1. Review TreeGuard settings early in deployment instead of assuming static/manual queue behavior.
2. If you want a narrower rollout, disable `all_nodes` and/or `all_circuits` and use allowlists first.
3. Validate behavior over multiple peak/off-peak windows.
4. If you want observation-only validation, set `dry_run = true` temporarily.
5. If you need fixed/manual behavior, set `enabled = false`.

## Overrides and Operational Notes

When enabled and not dry-run, TreeGuard may persist circuit SQM decisions to:

- `lqos_overrides.treeguard.json`

TreeGuard is designed to avoid fighting operator-owned overrides. If operator overrides exist for enrolled entities, TreeGuard skips those entities and reports warnings.

TreeGuard virtual-node decisions are runtime-only Bakery operations. They are not materialized back
into the base `network.json` by the scheduler, and they are not persisted as TreeGuard-owned
`set_node_virtual` entries in the effective shaping input. In v1 they are ephemeral: a daemon
restart returns the physical tree to the base operator-defined topology until TreeGuard decides
again.

TreeGuard circuit SQM decisions are also runtime overrides. The scheduler does not materialize TreeGuard-owned SQM changes back into the base `ShapedDevices.csv`, so clearing TreeGuard does not permanently rewrite operator-authored circuit SQM policy.

TreeGuard also refuses to manage nodes that are already marked `"virtual": true` in the base `network.json`. If stale legacy TreeGuard-owned node-virtualization overrides exist for those nodes, TreeGuard clears that legacy override state and falls back to the base topology definition.

For circuit SQM management, TreeGuard treats duplicate `device_id` values as unsafe identity collisions. If the same `device_id` appears in more than one circuit in `ShapedDevices.csv`, TreeGuard skips those affected circuits and clears any TreeGuard-owned SQM overrides for those duplicate device IDs.

If RTT telemetry is temporarily unavailable after a restart, TreeGuard does not treat missing RTT alone as evidence that it should revert `fq_codel` directions. Other guardrails such as utilization, QoO, and CPU pressure still apply.

TreeGuard also applies a conservative global per-tick circuit SQM change budget. On very large enrolled populations, excess circuit SQM changes are deferred to later ticks instead of stampeding Bakery in one pass.

For scale, TreeGuard no longer rebuilds circuit membership from `ShapedDevices.csv` on every circuit tick. It now keeps a cached per-circuit inventory derived from `ShapedDevices.csv`, reads per-circuit live telemetry from the shared once-per-second circuit rollup snapshot, and spreads large `all_circuits = true` SQM evaluations across multiple ticks instead of rescanning every enrolled circuit every second.

In practice, this means:

1. Link virtualization still follows the normal TreeGuard tick cadence.
2. Circuit SQM evaluation for small enrollments still completes quickly.
3. Very large `all_circuits` enrollments are swept incrementally over multiple ticks, with a target full sweep around 15 seconds instead of attempting a full per-second scan.
4. TreeGuard node virtualization now goes through Bakery live runtime planning/apply paths instead of forcing a LibreQoS reload or Bakery full reload.
5. Supported top-level runtime virtualization now uses a Bakery-side rebalance/migration plan that can promote child sites and direct circuits across queue roots while preserving the logical hierarchy for reporting.

Recent TreeGuard activity is available in two places:

- The WebUI TreeGuard status/activity views.
- The `lqosd` journal, where TreeGuard now logs each recorded activity event so reloads, override cleanup, SQM changes, and failures are diagnosable without websocket inspection.

## Related Pages

- [HTB + fq_codel + CAKE: Detailed Queueing Behavior](htb_fq_codel_cake.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [LibreQoS Backend Architecture](libreqos-backend-architecture.md)
- [Future Development Inputs](future-development-inputs.md)
