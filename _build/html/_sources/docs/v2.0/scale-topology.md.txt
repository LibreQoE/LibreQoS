# Scale Planning and Topology Design

This guide focuses on designing `network.json` and integration strategy choices for stable performance at scale.

## Core Design Principles

- Keep hierarchy only as deep as operationally necessary.
- Avoid concentrating too much traffic under one top-level parent.
- Prefer predictable naming and stable parent relationships to reduce queue churn.
- Validate topology changes in a maintenance window before broad rollout.

## Strategy Selection by Scale

Use the simplest integration strategy that still meets operational needs:

| Strategy | Typical Scale Fit | Tradeoff |
|---|---|---|
| `flat` | Maximum performance, minimal hierarchy | Lowest topology visibility/aggregation |
| `ap_only` | Large AP-driven networks | Good performance, moderate visibility |
| `ap_site` | Medium to large site/AP networks | Better aggregation with moderate overhead |
| `full` | Networks requiring full path/backhaul representation | Highest control and highest CPU/memory cost |

If `full` is required and you observe single-core saturation, use `promote_to_root` to distribute load.

## Parent/Child Distribution Guidance

Design goals:

- Keep top-level parents balanced in traffic and circuit count.
- Avoid very deep branch chains unless they add real shaping value.
- Keep sibling naming unique and stable (important for virtual-node promotion and operational clarity).

Warning signs:

- One core persistently overloaded while others are mostly idle.
- Frequent major queue rebuilds after small integration updates.
- WebUI CPU Tree shows skewed distribution not explained by demand.

## Virtual Nodes and Logical Grouping

Virtual nodes are useful for logical organization and aggregation views.

- Use virtual nodes to improve operability and reporting structure.
- Do not use virtual nodes as a substitute for physical topology clarity.
- Validate for name collisions after promotion behavior.

## Queue/Classifier Guardrails

At high scale, queue/class identifier pressure can become real.

- Monitor for urgent issues such as `TC_U16_OVERFLOW`.
- If encountered, reduce topology complexity and/or increase queue parallelism where appropriate.
- Reassess strategy depth (`full` -> `ap_site`/`ap_only`) when overflow risk appears.

See [Troubleshooting](troubleshooting.md#urgent-issue-codes-and-first-actions) for urgent-code response steps.

## Rollout Checklist for Topology Changes

1. Export current `network.json` and `ShapedDevices.csv` backups.
2. Apply one topology change set at a time.
3. Run/observe `lqos_scheduler` and `lqosd` logs after each change.
4. Validate WebUI:
   - CPU Tree / CPU Weights distribution
   - Flow Map/ASN/Tree behavior
   - scheduler status and urgent issues
5. Keep a rollback copy of prior integration settings and topology files.

## Related Pages

- [Integrations](integrations.md)
- [Performance Tuning](performance-tuning.md)
- [StormGuard](stormguard.md)
- [High Availability and Failure Domains](high-availability.md)
- [Configuration](configuration.md)
- [Troubleshooting](troubleshooting.md)
