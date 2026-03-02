# LibreQoS Backend Architecture and Queueing Design

This page explains how LibreQoS backend systems fit together at runtime:

1. Data-path packet handling (`XDP`/`eBPF` -> `tc` -> queue tree)
2. Queue hierarchy design (`mq` + `HTB` + leaf qdiscs)
3. AQM behavior (`fq_codel`, `CAKE`) and why benefits can appear below strict line rate
4. Control-path updates (Scheduler, `lqosd`, Bakery, incremental vs full reload)
5. Practical design boundaries for operators
 
## Source Context

This page incorporates details from our devblog posts:

- [Introducing the LibreQoS Bakery](https://devblog.libreqos.com/posts/0005-lqos-bakery/)
- [Fixing the Reload Penalty in LibreQoS](https://devblog.libreqos.com/posts/0013-no-more-locks/)


## 1) Backend Mental Model

LibreQoS has two cooperating planes:

1. Data plane:
   - classify packets quickly
   - map packets to queue classes
   - enforce fairness and latency behavior at line-rate
2. Control plane:
   - compute desired state from `network.json` and `ShapedDevices.csv`
   - apply the smallest safe set of queue changes
   - avoid unnecessary reload churn

In production terms:

- `XDP`/`eBPF` and lookup maps decide packet identity and CPU path.
- Linux traffic control (`tc`) enforces queueing policy.
- Bakery manages update deltas and reload boundaries.

## 2) Runtime Invariants

These invariants are useful for reasoning about whether backend behavior is healthy.

| Invariant | Why it matters | Symptom when broken | First check |
|---|---|---|---|
| Every shaped circuit maps to a valid hierarchy parent | No parent means no effective queue placement | Subscribers appear unshaped or bypass intended limits | Validate parent relationships in `network.json` and device input |
| Multi-queue root assumptions match NIC/runtime reality | CPU distribution depends on queue model consistency | One core saturated while others idle, unstable shaping at load | Verify NIC queue model and `mq`/class layout with `tc` output |
| Data-plane mapping is stable between XDP and `tc` | Mis-mapped packets cause wrong queue assignment | Unexpected class counters, mis-accounted traffic | Compare expected class IDs vs observed `tc -s` class counters |
| Control-plane changes stay within incremental-safe bounds | Reduces disruption from full-tree rebuilds | Frequent packet-impacting reload windows | Review change patterns: structural vs speed/mapping updates |
| Queue count stays within CPU/RAM budget | Leaf qdisc scale directly impacts resources | Memory growth, reload slowdowns, jitter under churn | Track queue count, RAM headroom, and update cadence |

## 3) End-to-End Packet and Control Path

```{mermaid}
flowchart LR
    subgraph DP[Data Plane]
      A[Ingress Packet] --> B[XDP parse: VLAN/PPPoE/IP/ports]
      B --> C[Flow cache and/or LPM mapping]
      C --> D[CPU steering via cpumap]
      D --> E[Metadata handoff to tc classifier]
      E --> F[tc class selection]
      F --> G[mq root]
      G --> H[HTB hierarchy]
      H --> I[Leaf qdisc: CAKE or fq_codel]
      I --> J[Egress]
    end

    subgraph CP[Control Plane]
      K[Scheduler inputs\nnetwork.json + ShapedDevices.csv] --> L[Desired state + command buffer]
      L --> M[lqosd command bus]
      M --> N[Bakery diff engine]
      N --> O[Incremental tc updates]
      N --> P[Controlled full reload]
    end

    N -. updates queue state .-> H
```

## 4) Data Plane Design

### 4.1 XDP and classification pipeline

LibreQoS performs early packet work in XDP where possible:

1. Parse packet headers once.
2. Resolve identity (flow/cache/LPM path).
3. Attach mapping metadata for downstream stages.

A key optimization direction has been reducing repeated lookups:

- use hot-cache hits for active addresses/flows
- fall back to LPM when needed
- avoid duplicate work between XDP and `tc` when metadata can be passed forward


### 4.2 Why `cpumap` is central

`cpumap` is used to spread work across cores so shaping does not bottleneck on a single queue path. This is a major part of scaling from "works" to "works at ISP traffic levels".

### 4.3 Cache, generation, and lock-pressure reduction

The high-level progression described in development notes/devblogs:

1. reduce full map wipes
2. move toward generation/epoch-style stale handling
3. reduce lock-heavy maintenance on hot paths

Operationally, this helps stabilize latency and CPU under frequent updates.


## 5) Queue Hierarchy: `mq` -> `HTB` -> leaf qdisc

LibreQoS queueing is intentionally layered:

1. `mq` root for multi-queue distribution
2. `HTB` for hierarchical rate envelopes
3. leaf qdisc (`CAKE` or `fq_codel`) for fairness/AQM behavior inside each envelope


```{mermaid}
flowchart TD
    A[mq root qdisc] --> B[HTB parent class CPU/RXQ 0]
    A --> C[HTB parent class CPU/RXQ 1]

    B --> D[HTB topology class: Site/AP/POP]
    D --> E[HTB circuit class: Subscriber/Circuit]
    E --> F[Leaf qdisc: CAKE or fq_codel]

    C --> G[HTB topology class: Site/AP/POP]
    G --> H[HTB circuit class: Subscriber/Circuit]
    H --> I[Leaf qdisc: CAKE or fq_codel]

    F --> J[Shaped egress packets]
    I --> J
```

### 5.1 HTB internals that matter in production

Important mechanics:

- Tokens: each packet consumes tokens based on size.
- Refill timing: token refill follows kernel timing (`jiffies`).
- `quantum`: bytes served before scheduler rotates class focus.
- `r2q`: influences derived quantum defaults and behavior.

Why operators care:

- very small or very large quantum values can affect fairness smoothness
- parent/child shaping relationships matter more than single-class tuning folklore
- HTB is the rate envelope; leaf qdiscs are not a drop-in replacement for HTB policy


## 6) AQM in LibreQoS: `fq_codel` and `CAKE`

### 6.1 Responsibility split

Practical split:

1. HTB: hierarchical bandwidth policy and limits
2. `fq_codel`/`CAKE`: queue fairness and delay control within that policy

### 6.2 Why gains can appear below strict line-rate events

AQM/fair-queueing improvements are not only about "100% saturated link" moments.

Even when the aggregate interface is not pinned continuously, user-visible gains can appear because:

1. microbursts still create queue pressure
2. competing flows still contend for queue service
3. per-flow scheduling reduces flow domination and queue-wait spikes
4. delay-oriented drop/mark behavior can prevent long-standing queues from building

So a safer operator claim is:

- "AQM can improve responsiveness and latency consistency under real mixed-load conditions, including periods below hard ceiling, depending on traffic mix and topology."

### 6.3 CAKE vs fq_codel in LibreQoS terms

General pattern:

1. Prefer CAKE when mixed-traffic smoothness and default behavior are the priority.
2. Prefer fq_codel when queue-count/resource pressure is dominant and observed QoE remains acceptable.
3. Re-test after major topology or queue-count changes.

Resource reality:

- both are flow-aware and keep state
- CAKE can have higher memory/CPU footprint in large queue populations

### 6.4 When below-line-rate gains may be limited

Lower latency under mixed load is common, but not guaranteed in every scenario.

Expect smaller gains when:

1. The bottleneck is outside the controlled queue path.
2. Traffic is sparse with little real queue contention.
3. Upstream/downstream shaping is applied in only one direction while the pain point is the opposite direction.
4. Hardware constraints force a queue design that cannot maintain enough isolation at peak moments.

Operator takeaway:

- Treat AQM gains as a result of queue dynamics and contention control, then validate empirically on your own traffic mix.


## 7) Bakery and Reload Behavior

Bakery exists to avoid unnecessary queue rebuilds and reduce reload penalties.

High-level flow:

1. Build desired state.
2. Diff desired vs active state.
3. Apply smallest safe delta.
4. Trigger full reload only when update type crosses live-mutation limits.


### 7.1 Lazy queueing and expiration

Key controls:

1. `lazy_queues`: defer creating parts of the hierarchy until active use.
2. `lazy_expire_seconds`: remove inactive queue state after timeout.

Practical effect:

- reduced memory overhead for dormant endpoints
- lower churn for large but partially active subscriber populations

### 7.2 Incremental vs reload boundary

| Change type | Usually incremental-safe | Often requires full reload | Why |
|---|---|---|---|
| Circuit IP-only change | Yes | No | Mapping update can often be applied without tree rebuild |
| Circuit/site speed change (subset) | Yes | Sometimes | Depends on structural impact and available class handles |
| Bulk all-circuit changes | Sometimes | Often | Scale and transaction/cardinality limits |
| Topology re-parent/restructure | Rarely | Yes | HTB subtree mutation constraints |
| Add/remove circuits | Yes (small/moderate) | Sometimes | Handle availability and diff correctness boundaries |

This table reflects Bakery design behavior and Linux `tc` mutation constraints discussed in the devblog material.

```{mermaid}
flowchart TD
    A[Config or integration change arrives] --> B[Build desired state and compute diff]
    B --> C{Any effective state change?}
    C -->|No| D[No-op]
    C -->|Yes| E{Structural hierarchy change?}
    E -->|Yes| F[Controlled full reload]
    E -->|No| G{Within incremental-safe limits?}
    G -->|Yes| H[Apply incremental tc updates]
    G -->|No| F
    H --> I[Verify class/qdisc state and counters]
    F --> I
```

### 7.3 Reload boundary quick rules

1. Prefer frequent small mapping/speed deltas over large structural churn.
2. Batch topology surgery into planned windows.
3. Expect higher risk when many circuits and many structure-affecting changes happen together.
4. Build operations cadence around incremental-safe updates by default.

## 8) Design Boundaries for Operators

### 8.1 Observability boundaries

| Signal | Strong for | Weak for |
|---|---|---|
| Queue counters and shaping metrics | Trend diagnosis, congestion behavior, policy validation | Exact per-packet causal proof |
| CAKE/fq_codel drops/marks | Detecting persistent queue pressure and policy effects | Full end-to-end application blame assignment |
| CPU/RAM and command timing | Capacity and reload risk planning | Isolating every microburst source |


### 8.2 Capacity risk factors and mitigations

| Risk factor | Typical symptom | Mitigation |
|---|---|---|
| Very high queue counts with CAKE everywhere | RAM growth and scheduler overhead | Use `lazy_queues`, expiry, selective fq_codel where appropriate |
| Frequent full-tree updates | Brief packet disruption windows | Increase incremental-safe update usage; batch structural changes |
| Incomplete parent mapping in hierarchy | Subscribers unexpectedly unshaped | Validate parent relationships in `network.json` and input data |
| Single-queue/weak NIC virtualization behavior | Poor spread and unstable shaping | Ensure multi-queue NIC path and verify queue mapping assumptions |

## 9) Symptom-to-Cause Troubleshooting Matrix

| Symptom | Common backend cause | First checks | Typical corrective direction |
|---|---|---|---|
| Latency spikes but interface throughput is not fully pegged | Microburst queue buildup, poor flow isolation, or direction mismatch | Compare latency vs queue/drop trends; verify both directions are shaped | Tune leaf qdisc strategy and verify directional shaping design |
| One CPU runs hot while others are underused | Queue steering imbalance or weak multi-queue path | Inspect CPU utilization and per-class counters by queue branch | Fix queue mapping assumptions and verify `mq`/class structure |
| Subscribers intermittently appear unshaped | Parent/hierarchy mapping mismatch | Validate parent node references and resulting class creation | Correct hierarchy mappings, then apply and verify class presence |
| Frequent short disruption during updates | Too many full-reload-triggering changes | Classify recent changes as structural vs incremental | Re-batch operations to favor incremental-safe deltas |
| RAM growth during scale-up | Too many active leaf qdiscs or aggressive CAKE footprint | Measure queue count and memory trends over update windows | Use lazy queue creation/expiry and consider selective fq_codel use |
| Dashboard traffic appears higher than expected user throughput | Counter scope differs from post-drop forwarded traffic | Compare dashboard metrics with `tc` drop/mark context | Align runbooks to metric semantics before escalating |

## 10) Change Validation Workflow

Use this lightweight workflow for backend-impacting changes.

### 10.1 Pre-change

1. Classify change type: mapping/speed/structure.
2. Estimate impact scope: number of affected circuits/classes.
3. Capture baseline:
   - latency trend
   - drop/mark behavior
   - CPU and RAM headroom
   - `tc` class/qdisc snapshot

### 10.2 During change

1. Watch control-plane behavior:
   - incremental apply vs full reload occurrence
   - command/runtime warnings or errors
2. Watch data-plane signals:
   - queue growth anomalies
   - directional latency drift
   - per-class counter discontinuities

### 10.3 Post-change

1. Re-check the same baseline signals.
2. Confirm hierarchy/class presence for changed circuits.
3. Verify subscriber-facing latency and throughput expectations.
4. If degraded, rollback or reduce change scope and re-apply in smaller batches.

### 10.4 Minimal command checklist

Adjust device names to your environment.

```bash
tc -s qdisc show dev <ifname>
tc -s class show dev <ifname>
journalctl -u lqosd --since "15 min ago"
```

## 11) Practical Tuning Sequence

Recommended order:

1. Validate topology hierarchy and parent mappings first.
2. Confirm queue counts and memory headroom.
3. Validate `mq`/multi-core spread behavior.
4. Choose CAKE vs fq_codel by observed QoE and resource budget.
5. Tune update cadence to favor incremental-safe changes.
6. Re-test after major speed-plan, topology, or integration-cadence changes.

## 12) Glossary

- `XDP`: earliest high-performance packet hook in Linux.
- `eBPF`: in-kernel programmable packet processing.
- `LPM`: longest-prefix-match lookup for identity mapping.
- `cpumap`: XDP map for steering processing to CPUs.
- `tc`: Linux traffic-control subsystem.
- `qdisc`: queue discipline object in `tc`.
- `mq`: multi-queue root structure.
- `HTB`: hierarchical token bucket scheduler/shaper.
- `fq_codel`: fair queueing + CoDel delay control.
- `CAKE`: integrated shaper/fairness/AQM qdisc.
- `Bakery`: LibreQoS state-diff and incremental update subsystem.
- `epoch/generation`: state-aging approach used to reduce lock-heavy global clears.

## Related Reading

- [CAKE](cake.md)
- [Performance Tuning](performance-tuning.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Best Practices Guide for ISP Operations](best-practices.md)
- [Deployment Recipes](recipes.md)
- [Introducing the LibreQoS Bakery (Herbert)](https://devblog.libreqos.com/posts/0005-lqos-bakery/)
- [Fixing the Reload Penalty in LibreQoS (Herbert)](https://devblog.libreqos.com/posts/0013-no-more-locks/)
