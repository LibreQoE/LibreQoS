# LibreQoS Backend Architecture and Queueing Design

This page explains how LibreQoS backend systems fit together at runtime:

1. Data-path packet handling (`XDP`/`eBPF` -> `tc` -> queue tree)
2. Queue hierarchy design (`mq` + `HTB` + leaf qdiscs)
3. AQM behavior (`fq_codel`, `CAKE`) and why benefits can appear below strict line rate
4. Control-path updates (Scheduler, `lqosd`, Bakery, incremental vs full reload)
5. Practical design boundaries for operators

For a full queueing deep-dive, see [HTB + fq_codel + CAKE: Detailed Queueing Behavior](htb_fq_codel_cake.md).
 
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
- Queue mode (`shape` vs `observe`) controls whether the subscriber shaping tree is active or intentionally removed for baseline measurement.

## 2) Runtime Invariants

These invariants are useful for reasoning about whether backend behavior is healthy.

| Invariant | Why it matters | Symptom when broken | First check |
|---|---|---|---|
| Every shaped circuit maps to a valid hierarchy parent | No parent means no effective queue placement | Subscribers appear unshaped or bypass intended limits | Validate parent relationships in `network.json` and device input |
| Multi-queue root assumptions match NIC/runtime reality | CPU distribution depends on queue model consistency | One core saturated while others idle, unstable shaping at load | Verify NIC queue model and `mq`/class layout with `tc` output |
| Data-plane mapping is stable between XDP and `tc` | Mis-mapped packets cause wrong queue assignment | Unexpected class counters, mis-accounted traffic | Compare expected class IDs vs observed `tc -s` class counters |
| Control-plane changes stay within incremental-safe bounds | Reduces disruption from full-tree rebuilds | Frequent packet-impacting reload windows | Review change patterns: structural vs speed/mapping updates |
| Queue count stays within CPU/RAM budget | Leaf qdisc scale directly impacts resources | Memory growth, reload slowdowns, jitter under churn | Track queue count, RAM headroom, and update cadence |
| Queue mode and dataplane mappings are sequenced consistently | Packets must not be steered toward removed queue state | Brief outages, stale class targets, mis-accounted traffic | Check queue mode, IP mapping lifecycle, and live `tc` state together |

## 3) Runtime Authority and Configuration Model

LibreQoS has both on-disk config files and a long-running runtime daemon, but they are not the same authority at every moment.

1. `lqosd` is the runtime control-plane authority while it is running.
2. UI/config API updates go through `lqosd`, which updates in-memory state and then drives apply/reload behavior.
3. Manual edits to `/etc/lqos.conf` are operator-managed source inputs, but they do not automatically become active runtime state until the daemon reload path consumes them.

Operational takeaway:

- treat the UI/config API path as the authoritative live-update workflow
- treat direct file edits as a separate operator action that still needs a runtime reload/apply boundary
- do not assume “file changed” and “runtime desired state changed” are equivalent at the same instant

## 4) End-to-End Packet and Control Path

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

## 5) Data Plane Design

### 5.1 XDP and classification pipeline

LibreQoS performs early packet work in XDP where possible:

1. Parse packet headers once.
2. Resolve identity (flow/cache/LPM path).
3. Attach mapping metadata for downstream stages.

A key optimization direction has been reducing repeated lookups:

- use hot-cache hits for active addresses/flows
- fall back to LPM when needed
- avoid duplicate work between XDP and `tc` when metadata can be passed forward


### 5.2 Why `cpumap` is central

`cpumap` is used to spread work across cores so shaping does not bottleneck on a single queue path. This is a major part of scaling from "works" to "works at ISP traffic levels".

### 5.3 Cache, generation, and lock-pressure reduction

The high-level progression described in development notes/devblogs:

1. reduce full map wipes
2. move toward generation/epoch-style stale handling
3. reduce lock-heavy maintenance on hot paths

Operationally, this helps stabilize latency and CPU under frequent updates.

### 5.4 Mapping state is part of the data-path contract

Queue classes and IP mappings are related but not identical backend state.

1. The shaping tree defines where packets can land in `tc`.
2. IP mappings define which circuit/class packets are currently steered toward.
3. During ordinary `shape -> shape` updates, LibreQoS tries to preserve stable mappings and stable queue handles where safe.
4. During disruptive transitions, especially `observe <-> shape`, LibreQoS can intentionally clear and later republish mappings so packets are not pointed at queue state that no longer exists.

This sequencing matters as much as the queue commands themselves. A healthy backend design must reason about queue-tree changes and mapping changes together.


## 6) Queue Hierarchy: `mq` -> `HTB` -> leaf qdisc

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

### 6.1 HTB internals that matter in production

Important mechanics:

- Tokens: each packet consumes tokens based on size.
- Refill timing: token refill follows kernel timing (`jiffies`).
- `quantum`: bytes served before scheduler rotates class focus.
- `r2q`: influences derived quantum defaults and behavior.

Why operators care:

- very small or very large quantum values can affect fairness smoothness
- parent/child shaping relationships matter more than single-class tuning folklore
- HTB is the rate envelope; leaf qdiscs are not a drop-in replacement for HTB policy


## 7) AQM in LibreQoS: `fq_codel` and `CAKE`

### 7.1 Responsibility split

Practical split:

1. HTB: hierarchical bandwidth policy and limits
2. `fq_codel`/`CAKE`: queue fairness and delay control within that policy

### 7.2 Why AQM still helps even below line-rate saturation

Even when a link is not saturated, fq_codel and CAKE still affect packet behavior. The drop logic (CoDel/BLUE/COBALT) usually stays idle, but the fair-queueing scheduler continues to interleave flows, preventing bursts from one flow from monopolizing the serialization point of the link. This keeps latency-sensitive traffic responsive.

Let's think about the way these work:

A packet is enqueued for sending (from any source). It goes into TC and is matched to the SQM qdisc.

A flow key is generated (and the tin determined if using CAKE with diffserv). The enqueue time is recorded so we know how long the packet has been waiting.

With both fq_codel and CAKE, the packet is now sitting in a queue specific to that flow (how specific depends on configuration and hashing).

Now dequeue happens – the interface indicates that it can accept more packets. When the link is not saturated, this tends to happen quickly.

fq_codel and CAKE then schedule packets between flows (conceptually round-robin using deficit scheduling; CAKE also applies tin priority).

At dequeue time, the sojourn time (time spent in the queue) is evaluated. In congested conditions this can trigger the CoDel or BLUE/COBALT drop logic. However, when the link is not saturated those mechanisms are rarely active.

However, the flow queueing itself still matters.

Packets are drawn from multiple flow queues in a fair order before reaching the device queue. At the physical layer the link ultimately serializes traffic one bit at a time, so the order packets reach that serialization point still affects latency behavior.

Even without sustained congestion:

- Responsiveness of well-behaved flows remains stable. Short control packets (DNS, SSH, TCP ACKs, etc.) are less likely to get stuck behind bursts from another flow.

- Burstiness is reduced before packets hit the serialization point. Instead of one flow dumping a large burst, flows are interleaved by the scheduler.

So even when the AQM drop logic is mostly idle, the fair-queueing part of SQM is still doing useful work by controlling how packets reach the wire.

### 7.3 CAKE vs fq_codel in LibreQoS terms

General pattern:

1. Prefer CAKE when mixed-traffic smoothness and default behavior are the priority.
2. Prefer fq_codel when queue-count/resource pressure is dominant and observed QoE remains acceptable.
3. Re-test after major topology or queue-count changes.

Resource reality:

- both are flow-aware and keep state
- CAKE can have higher memory/CPU footprint in large queue populations

### 7.4 When below-line-rate gains may be limited

Lower latency under mixed load is common, but not guaranteed in every scenario.

Expect smaller gains when:

1. The bottleneck is outside the controlled queue path.
2. Traffic is sparse with little real queue contention.
3. Upstream/downstream shaping is applied in only one direction while the pain point is the opposite direction.
4. Hardware constraints force a queue design that cannot maintain enough isolation at peak moments.

Operator takeaway:

- Treat AQM gains as a result of queue dynamics and contention control, then validate empirically on your own traffic mix.


## 8) Bakery and Reload Behavior

Bakery exists to avoid unnecessary queue rebuilds and reduce reload penalties.

High-level flow:

1. Build desired state.
2. Diff desired vs active state.
3. Apply smallest safe delta.
4. Trigger full reload when a change is outside live-mutation support, or when runtime verification/drift detection marks incremental topology mutation unsafe.


### 8.1 Lazy queueing and expiration

Key controls:

1. `lazy_queues`: defer creating parts of the hierarchy until active use.
2. `lazy_expire_seconds`: remove inactive queue state after timeout.

Practical effect:

- reduced memory overhead for dormant endpoints
- lower churn for large but partially active subscriber populations

### 8.2 Incremental vs reload boundary

| Change type | Usually incremental-safe | Often requires full reload | Why |
|---|---|---|---|
| Circuit IP-only change | Yes | No | Mapping updates can usually be applied without rebuilding the queue tree |
| Circuit SQM-only change | Yes | No | Leaf qdisc kind/parameter changes can usually be applied live |
| Circuit/site speed change (subset) | Yes | Sometimes | Depends on structural impact, queue-count pressure, and available class handles |
| Ordinary circuit parent move | Yes | Sometimes | Bakery uses staged live migration for common active parent/class moves, including qdisc-handle rotation and final-state verification, but it still escalates to reload if the migration cannot be applied or verified safely |
| TreeGuard runtime node virtualization (supported subtree/top-level rebalance path) | Yes | Sometimes | Bakery can apply supported runtime virtualization live, but deferred cleanup, live-state verification failures, or accumulated dirty runtime subtrees can mark `reload required` and freeze further incremental topology mutation until a full reload |
| Bulk all-circuit changes | Sometimes | Often | Scale and transaction/cardinality limits still matter, even with better incremental behavior |
| Site add/remove or broader structural topology change | Rarely | Yes | HTB subtree mutation constraints remain much stricter at site/topology level than at per-circuit level |
| Add/remove circuits | Yes (small/moderate) | Sometimes | Handle availability, tree size, and diff correctness boundaries |

This table reflects Bakery design behavior and Linux `tc` mutation constraints discussed in the devblog material.

```{mermaid}
flowchart TD
    A[Config or runtime change arrives] --> B[Build desired or runtime target state]
    B --> C{Any effective state change?}
    C -->|No| D[No-op]
    C -->|Yes| E{Supported live mutation path?}
    E -->|No| F[Controlled full reload]
    E -->|Yes| G[Apply staged incremental/runtime mutation]
    G --> H{Live verification and cleanup safe?}
    H -->|Yes| I[Keep incremental state authoritative]
    H -->|No| J[Mark reload required and freeze further incremental topology mutation]
    J --> F
    F --> K[Re-establish single authoritative queue model]
```

### 8.3 Queue modes and transition semantics

LibreQoS currently has two explicit queue modes:

1. `shape`
   - normal shaping mode
   - root `mq` present
   - HTB hierarchy present
   - leaf qdiscs present
   - per-circuit IP mappings present
2. `observe`
   - true-baseline mode
   - root `mq` retained
   - subscriber shaping tree removed
   - per-circuit IP mappings cleared before teardown
   - mappings republished after returning to `shape`

Important operator boundary:

- `observe` is intentionally honest, not hitless
- switching `observe <-> shape` can briefly interrupt traffic because the shaping tree really is removed and later rebuilt
- this is different from ordinary `shape -> shape` updates, where LibreQoS tries to preserve stable handles and queue placement where possible

### 8.4 Retained-root full reload behavior

Current Bakery full reloads try to avoid unnecessary root churn.

1. Verify live kernel `tc` state, not just planned state.
2. If the root `mq` is healthy and matches the expected layout, retain it.
3. Prune child qdiscs beneath the retained root and verify the subtree is clean.
4. Rebuild the shaping tree beneath that retained root.
5. Fall back to root recovery only when retained-root reuse is unsafe or verification fails.

This retained-root strategy reduces avoidable root-level churn, while still preferring explicit recovery when live state is ambiguous.

### 8.5 Reload boundary quick rules

1. Prefer frequent small mapping/speed deltas over large structural churn.
2. Batch topology surgery into planned windows.
3. Expect higher risk when many circuits and many structure-affecting changes happen together.
4. Build operations cadence around incremental-safe updates by default.

### 8.6 Full-reload safety guards

Current Bakery full reloads apply two conservative safety checks before and during large queue rebuilds:

1. A qdisc preflight estimates planned qdiscs per interface and also separates infrastructure, `cake`, and `fq_codel` leaf qdiscs.
2. That same preflight applies a conservative memory forecast and hard-blocks clearly unsafe full reloads before `tc -batch` starts.
3. During chunked full reload apply, Bakery re-checks host memory at chunk boundaries and aborts the remaining apply if available memory drops below its safety floor.
4. These guards are intentionally biased toward false positives on large reloads so the system fails early with diagnostics instead of spiraling into an OOM event.

### 8.7 Runtime safety model

Bakery's newer runtime-safety direction is intentionally narrow:

1. Reconcile enough live state to decide whether deferred cleanup is safe.
2. Detect material drift between Bakery's intended state and live kernel `tc` state.
3. Stop trusting incremental/runtime mutation once drift is real.
4. Escalate to a controlled full reload as the recovery path.

This is intentionally **not** a broad self-healing reconciler. LibreQoS is biased toward:

- lightweight cleanup gating for expected lag
- explicit `reload required` escalation on material live-state drift
- one controlled full reload to re-establish a single authoritative queue model

That design keeps failure handling easier to reason about than trying to incrementally repair arbitrary split-brain queue state.

### 8.8 Explicit qdisc-handle management

Bakery now treats leaf qdisc handles as persistent runtime state rather than disposable auto-allocation details.

1. Circuit leaf qdiscs are assigned explicit handle majors and persisted across applies.
2. Handle assignments rotate when a live mutation changes the effective leaf qdisc kind or qdisc parent.
3. Full reload planning reserves live handle majors so rebuilds do not collide with surviving kernel state.
4. Parent-changed live migration is rejected if Bakery detects stale-handle reuse that would make final state ambiguous.

This handle model is one of the mechanisms that makes common live circuit migration safer than earlier Bakery generations.

### 8.9 Runtime virtualization limits and operator expectations

Current runtime virtualization support is intentionally constrained.

1. Non-top-level runtime virtualization is limited to same-queue / same-major-domain subtree paths.
2. Top-level runtime virtualization uses a separate rebalance/promote path and only applies when Bakery can derive a deterministic split.
3. Runtime operations may remain in `AppliedAwaitingCleanup` while deferred prune work completes.
4. Runtime operations can become `Dirty`; repeated dirty subtree states escalate to `reload required` rather than attempting broad self-healing.

Operator takeaway:

- treat runtime virtualization as a narrow live-mutation feature with verification gates
- treat `reload required` as the authoritative signal that Bakery no longer trusts incremental topology mutation

## 9) Design Boundaries for Operators

### 9.1 Observability boundaries

| Signal | Strong for | Weak for |
|---|---|---|
| Queue counters and shaping metrics | Trend diagnosis, congestion behavior, policy validation | Exact per-packet causal proof |
| CAKE/fq_codel drops/marks | Detecting persistent queue pressure and policy effects | Full end-to-end application blame assignment |
| CPU/RAM and command timing | Capacity and reload risk planning | Isolating every microburst source |

### 9.2 Metric sample semantics and clock domains

Backend metrics do not all come from the same sampling source or clock edge.

1. Some values are sampled from queue/kernel state.
2. Some values are sampled from flow telemetry.
3. Some rollups are built from canonical raw samples and only later rendered as percentages.

Practical implication:

- do not casually combine unrelated numerator/denominator sources and assume they describe the same exact second
- prefer transporting canonical samples/counts and deriving percentages at presentation time
- treat “same field name” and “same clock domain” as separate questions

This matters especially for retransmit, packet, and rate-derived health metrics.

### 9.3 Capacity risk factors and mitigations

| Risk factor | Typical symptom | Mitigation |
|---|---|---|
| Very high queue counts with CAKE everywhere | RAM growth and scheduler overhead | Use `lazy_queues`, expiry, selective fq_codel where appropriate |
| Frequent full-tree updates | Brief packet disruption windows | Increase incremental-safe update usage; batch structural changes, and let Bakery keep ordinary circuit moves incremental where possible |
| Incomplete parent mapping in hierarchy | Subscribers unexpectedly unshaped | Validate parent relationships in `network.json` and input data |
| Single-queue/weak NIC virtualization behavior | Poor spread and unstable shaping | Ensure multi-queue NIC path and verify queue mapping assumptions |

## 10) Symptom-to-Cause Troubleshooting Matrix

| Symptom | Common backend cause | First checks | Typical corrective direction |
|---|---|---|---|
| Latency spikes but interface throughput is not fully pegged | Microburst queue buildup, poor flow isolation, or direction mismatch | Compare latency vs queue/drop trends; verify both directions are shaped | Tune leaf qdisc strategy and verify directional shaping design |
| One CPU runs hot while others are underused | Queue steering imbalance or weak multi-queue path | Inspect CPU utilization and per-class counters by queue branch | Fix queue mapping assumptions and verify `mq`/class structure |
| Subscribers intermittently appear unshaped | Parent/hierarchy mapping mismatch | Validate parent node references and resulting class creation | Correct hierarchy mappings, then apply and verify class presence |
| Frequent short disruption during updates | Too many full-reload-triggering changes or runtime drift escalation | Classify recent changes as structural vs incremental, and check for `reload required` events | Re-batch operations to favor incremental-safe deltas and investigate live-state drift |
| RAM growth during scale-up | Too many active leaf qdiscs or aggressive CAKE footprint | Measure queue count and memory trends over update windows | Use lazy queue creation/expiry and consider selective fq_codel use |
| Dashboard traffic appears higher than expected user throughput | Counter scope differs from post-drop forwarded traffic | Compare dashboard metrics with `tc` drop/mark context | Align runbooks to metric semantics before escalating |

## 11) Change Validation Workflow

Use this lightweight workflow for backend-impacting changes.

### 11.1 Pre-change

1. Classify change type: mapping/speed/structure.
2. Estimate impact scope: number of affected circuits/classes.
3. Capture baseline:
   - latency trend
   - drop/mark behavior
   - CPU and RAM headroom
   - `tc` class/qdisc snapshot

### 11.2 During change

1. Watch control-plane behavior:
   - incremental apply vs full reload occurrence
   - command/runtime warnings or errors
2. Watch data-plane signals:
   - queue growth anomalies
   - directional latency drift
   - per-class counter discontinuities

### 11.3 Post-change

1. Re-check the same baseline signals.
2. Confirm hierarchy/class presence for changed circuits.
3. Verify subscriber-facing latency and throughput expectations.
4. If degraded, rollback or reduce change scope and re-apply in smaller batches.

For `observe <-> shape` transitions, add two specific checks:

1. confirm the queue mode actually matches the intended state
2. confirm per-circuit mappings were cleared or republished at the right phase of the transition

### 11.4 Minimal command checklist

Adjust device names to your environment.

```bash
tc -s qdisc show dev <ifname>
tc -s class show dev <ifname>
journalctl -u lqosd --since "15 min ago"
```

## 12) Practical Tuning Sequence

Recommended order:

1. Validate topology hierarchy and parent mappings first.
2. Confirm queue counts and memory headroom.
3. Validate `mq`/multi-core spread behavior.
4. Choose CAKE vs fq_codel by observed QoE and resource budget.
5. Tune update cadence to favor incremental-safe changes.
6. Re-test after major speed-plan, topology, or integration-cadence changes.

## 13) Glossary

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

- [HTB + fq_codel + CAKE: Detailed Queueing Behavior](htb_fq_codel_cake.md)
- [HTB + fq-codel / CAKE](htb_fq_codel_cake.md)
- [Performance Tuning](performance-tuning.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Best Practices Guide for ISP Operations](best-practices.md)
- [Deployment Recipes](recipes.md)
- [Introducing the LibreQoS Bakery](https://devblog.libreqos.com/posts/0005-lqos-bakery/)
- [Fixing the Reload Penalty in LibreQoS](https://devblog.libreqos.com/posts/0013-no-more-locks/)
