# Best Practices Guide for ISP Operations

This guide consolidates operational best practices for deploying and running LibreQoS in ISP environments.

Use this page as an operations playbook. Use feature-specific pages for implementation details.

## How to Use This Guide

1. Pick your operating model with the decision tables.
2. Complete the pre-deployment checklist before first production cutover.
3. Use the runbooks during maintenance windows and incidents.
4. Use the day-2 checklist as your recurring operational cadence.

## 1) Purpose and Audience

This guide is for:

- ISP owners and technical managers
- Network engineers and architects
- NOC/support operators

This guide is not a replacement for installation and component documentation. Start with [Quickstart](quickstart.md), then use this page to standardize real-world operations.

## 2) Quick Decision Matrix

### Decision Table A: Source of Truth

| If this describes your operation | Recommended model | Why |
|---|---|---|
| You use a supported CRM/NMS integration end-to-end | Built-in integration mode | Lowest manual drift and simpler recurring updates |
| You have an internal orchestration pipeline | Custom source of truth mode | Preserves your automation while keeping ownership explicit |
| You are small and intentionally manage files directly | Manual files mode | Acceptable only when change volume is low and discipline is high |

Reference: [Operating Modes and Source of Truth](operating-modes.md), [CRM/NMS Integrations](integrations.md)

### Decision Table B: Topology Strategy

| Operational requirement | Recommended strategy |
|---|---|
| Maximum performance, minimal hierarchy | `flat` |
| AP-level aggregation with better performance headroom | `ap_only` |
| Site/AP visibility with moderate overhead | `ap_site` |
| Full backhaul/path hierarchy is required | `full` |

Reference: [Scale Planning and Topology Design](scale-topology.md)

### Decision Table C: Deployment Substrate

| Deployment choice | Choose when | Main caution |
|---|---|---|
| Bare metal | Production-critical throughput and lowest latency overhead are required | Validate NIC support and single-thread CPU performance |
| VM (for example Proxmox) | You already operate mature virtualization and throughput targets fit VM envelope | Account for virtualization overhead and align virtio multiqueue to vCPU |

Reference: [System Requirements](requirements.md), [Recipe: Proxmox VM Deployment](recipes-proxmox-vm.md)

### Feature Coverage Matrix

| Feature Area | Operational Best-Practice Focus | Primary Reference |
|---|---|---|
| Integrations | Single source-of-truth ownership, overwrite discipline, controlled parameter changes | [Integrations](integrations.md) |
| Topology Strategies | Right-size hierarchy depth, balance parent distribution, maintain stable naming | [Scale Planning and Topology Design](scale-topology.md) |
| High Availability and Bypass | Deterministic active/backup routing policy, recurring failover/failback drills | [High Availability and Failure Domains](high-availability.md) |
| StormGuard | Use as adaptive protection where WAN conditions vary, validate behavior during events | [StormGuard](stormguard.md) |
| Node Manager UI | Verify operational state after changes; distinguish path failures from UI-only symptoms | [Node Manager UI](node-manager-ui.md) |
| Deployment Recipes | Apply proven implementation patterns for real-world topology scenarios | [Deployment Recipes](recipes.md) |

## 3) Pre-Deployment Best Practices

1. Validate platform fit before cabling.
   - Supported NIC family
   - Sufficient single-thread CPU performance
   - RAM sized for expected subscriber/circuit scale

2. Validate baseline service health before pilot traffic.

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```

3. Validate source-of-truth ownership before first sync.
   - Decide whether integrations, external scripts, or manual files own persistence
   - Avoid mixed ownership from day one (the most common cause of configuration drift)

4. Validate integration data hygiene before production shaping.
   - Duplicate IP checks
   - ParentNode consistency checks
   - `allow_subnets` and `ignore_subnets` scope checks

### Checklist: Pre-Deploy Readiness

- [ ] Platform meets supported NIC/CPU/RAM requirements
- [ ] `lqosd` and `lqos_scheduler` are healthy
- [ ] Source-of-truth owner is explicitly documented
- [ ] Integration credentials and sync settings are validated
- [ ] Data hygiene checks are clean (duplicates/parents/subnets)
- [ ] Rollback path is documented before first cutover

## 4) Source-of-Truth and Integration Best Practices

1. Enforce one durable owner for shaping data.
   - Built-in integration mode: integration jobs own regenerated files
   - Custom mode: your external system owns generated files
   - Manual mode: direct edits are the durable path

2. Treat overwrite behavior as an explicit design choice.
   - If integrations own topology, use overwrite behavior intentionally
   - Do not rely on manual edits to generated files unless ownership policy explicitly allows it

3. Change integration parameters one set at a time.
   - Save changes
   - Restart scheduler if required by your workflow
   - Validate logs and WebUI state before applying additional changes

4. Use recurring scheduler behavior intentionally.
   - Faster refresh intervals increase control-plane churn
   - Slower intervals reduce churn but delay corrections

Reference: [Integrations](integrations.md), [Configuration](configuration.md), [Troubleshooting](troubleshooting.md)

## 5) Topology and Scale Best Practices

1. Keep hierarchy only as deep as operationally necessary.
2. Balance parent distribution to avoid single-core concentration.
3. Favor stable naming and parent relationships to reduce queue churn.
4. For multi-edge environments, prefer explicit, operator-controlled path policy over inferred assumptions.
5. Validate topology changes in a maintenance window, not ad hoc during peak load.

Field pattern:

- Operators commonly recover performance and stability by moving from unnecessary `full` hierarchy depth to `ap_site` or `flat` when full path control is not required.

Reference: [Scale Planning and Topology Design](scale-topology.md), [Recipes](recipes.md)

## 6) Performance and Capacity Best Practices

1. Design around single-thread performance, not only total core count.
2. Verify queue/CPU distribution after topology or strategy changes.
3. In VM deployments, align virtio multiqueue with vCPU and verify under realistic peak load.
4. Treat MTU/encapsulation mismatches as first-class suspects in throughput anomalies.
5. Use capacity planning discipline before symptoms force emergency hardware changes.

Reference: [System Requirements](requirements.md), [Performance Tuning](performance-tuning.md)

## 7) High Availability, Bypass, and Maintenance Best Practices

1. Use deterministic active/backup policy (OSPF/BGP) for failover and failback.
2. In switch-centric designs, validate shaped path and bypass path behavior independently.
3. Keep failover drills routine; do not wait for incidents to test convergence behavior.
4. Ensure backup paths are sized for realistic degraded-state demand.

### Runbook: Maintenance Cutover Validation

1. Confirm backup path health and expected capacity.
2. Record current service and path state.
3. Shift preference to backup policy.
4. Validate subscriber traffic continuity plus key latency/throughput indicators.
5. Execute maintenance on primary path.
6. Restore primary preference.
7. Validate failback behavior and post-change stability.
8. Document outcome and any required corrections.

### Maintenance Cutover Sequence (Visual)

```{mermaid}
flowchart TD
    A[Confirm backup path health and capacity] --> B[Record current service and path state]
    B --> C[Shift routing preference to backup path]
    C --> D{Traffic continuity and KPIs healthy?}
    D -->|No| E[Stop and remediate before maintenance]
    D -->|Yes| F[Perform maintenance on primary path]
    F --> G[Restore primary path preference]
    G --> H{Failback stable?}
    H -->|No| I[Investigate and hold on backup/controlled state]
    H -->|Yes| J[Document closure and outcomes]
```

Reference: [High Availability and Failure Domains](high-availability.md), [Recipe: Switch-Centric Fabric](recipes-switch-fabric-sdwan.md)

## 8) Monitoring and Incident Response Best Practices

1. Start triage with service health and logs, then move to topology and integration.
2. Distinguish shaping-path failures from telemetry/UI presentation issues.
3. Capture reproducible evidence before making broad corrective changes.
4. Keep a standard incident evidence bundle for escalation.

Standard evidence bundle:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd --since "30 minutes ago"
journalctl -u lqos_scheduler --since "30 minutes ago"
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```

### Runbook: Shaping Not Applied or Coverage Drops

1. Confirm services are healthy.
2. Check scheduler logs for validation failures.
3. Check for duplicate IP, parent mismatch, or subnet-scope misalignment.
4. Confirm source-of-truth ownership (manual edits vs integration regeneration).
5. Re-run scheduler refresh after corrections.
6. Validate shaped/unshaped trend and any high-priority impacted subscribers.
7. If unresolved, collect the evidence bundle and escalate with timestamps and recent config changes.

### Incident Triage Flow (Visual)

```{mermaid}
flowchart TD
    A[Check lqosd and lqos_scheduler service health] --> B{Services healthy?}
    B -->|No| C[Restore service health and re-check]
    B -->|Yes| D[Review scheduler logs for validation failures]
    D --> E{Data hygiene issues present?}
    E -->|Yes| F[Correct duplicate IP, parent mismatch, subnet scope]
    E -->|No| G[Confirm source-of-truth ownership]
    F --> H[Re-run scheduler refresh]
    G --> H
    H --> I{Shaping coverage restored?}
    I -->|Yes| J[Validate subscriber impact and close]
    I -->|No| K[Collect evidence bundle and escalate]
```

Reference: [Troubleshooting](troubleshooting.md), [Integrations](integrations.md)

## 9) Change Management Best Practices

1. Use pilot-first progression for strategy and topology changes.
2. Apply one change set per window and validate before next set.
3. Always preserve rollback artifacts before major changes.
4. Log change intent, execution, validation evidence, and closure.

### Checklist: Change Window Execution

- [ ] Scope and success criteria are defined
- [ ] Rollback plan and artifacts are ready
- [ ] One change set only (no mixed experiments)
- [ ] Post-change validation completed and recorded
- [ ] Escalation path and owner are defined before closure

### Checklist: Day-2 Operations Cadence

- [ ] Daily: service health + urgent issues reviewed
- [ ] Daily: shaped/unshaped trend reviewed for regressions
- [ ] Weekly: topology and parent distribution sanity review
- [ ] Weekly: scheduler/log anomaly review
- [ ] Monthly: capacity/headroom and hardware-fit review

## 10) Common Anti-Patterns to Avoid

1. Competing sources of truth for shaping inputs.
2. Treating unsupported NICs as production-safe.
3. Defaulting to deep hierarchy without operational need.
4. Making failover assumptions without explicit validation.
5. Mixing multiple major changes in one maintenance window.
6. Ignoring data hygiene errors (duplicates, parent mismatch, subnet mis-scope).

## 11) MikroTik RouterOS v7 Practical Notes

These are operational notes, not full router design guidance.

1. Keep routing policy deterministic between shaped and bypass paths.
2. Keep interface naming and policy mapping consistent with your LibreQoS runbooks.
3. Validate failover/failback behavior under maintenance conditions, not only in theory.
4. For multi-WAN/PCC environments, avoid ambiguous path ownership; ensure each subscriber flow has a predictable shaping path.

Conceptual policy example (cost preference pattern):

```text
/routing ospf interface-template
add interfaces=vlan-primary-path area=backbone-v2 cost=10
add interfaces=vlan-bypass-path area=backbone-v2 cost=200
```

Reference: [Recipe: Switch-Centric Fabric](recipes-switch-fabric-sdwan.md)

## 12) NLNet Verification Mapping (Milestone 10a)

This guide satisfies `10a Best practices guide` by providing:

1. End-to-end operational decision framework (source of truth, topology strategy, substrate choice).
2. Actionable checklists for pre-deploy, change windows, and day-2 operations.
3. Incident response runbooks for common operational failure classes.
4. Field-aligned anti-pattern guidance derived from real deployment behavior.
5. Cross-linked references to detailed implementation docs and recipes.

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [System Requirements](requirements.md)
- [Performance Tuning](performance-tuning.md)
- [High Availability and Failure Domains](high-availability.md)
- [Deployment Recipes](recipes.md)
- [Case Studies](case-studies.md)
- [Troubleshooting](troubleshooting.md)
