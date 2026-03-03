# HTB + fq_codel + CAKE: Detailed Queueing Behavior in LibreQoS

This page is the canonical queueing deep-dive companion to [LibreQoS Backend Architecture](libreqos-backend-architecture.md).

It explains:

1. Why LibreQoS layers `HTB` with leaf qdiscs (`fq_codel` or `CAKE`)
2. How `fq_codel` works in practice
3. How `CAKE` works in practice
4. When to choose `fq_codel` vs `CAKE`
5. Operator troubleshooting and observability patterns

## 1) Why These Three Components Exist Together

In production LibreQoS:

1. `HTB` provides hierarchical rate policy (`rate`, `ceil`, borrow, hierarchy)
2. `fq_codel` or `CAKE` provides per-flow queue service and AQM behavior inside each shaped envelope

This split is intentional:

- Policy problem: "How much can this class send?" -> `HTB`
- Queueing problem: "Which packet should send next while controlling latency?" -> `fq_codel`/`CAKE`

## 2) Runtime Placement in LibreQoS

Conceptually, packets pass through:

1. `mq` root
2. `HTB` hierarchy
3. leaf qdisc per shaped class (`CAKE` by default, `fq_codel` optional)

Operationally, this is commonly:

`mq` root -> per-CPU HTB parents -> per-circuit HTB classes -> leaf qdisc (`cake diffserv4` or `fq_codel`)

Each HTB class has a child qdisc attachment point. If no explicit leaf qdisc is attached, behavior falls back to kernel default queueing for that class.

Practical LibreQoS behavior model:

1. Default behavior uses `HTB` + `cake diffserv4` for shaped circuits.
2. TreeGuard (upcoming feature) can dynamically switch circuit directions between `cake diffserv4` and `fq_codel` based on low-load/RTT guardrails.
3. TreeGuard is not enabled by default.

See [TreeGuard (Upcoming v2.0 Feature)](treeguard.md) for configuration and rollout details.

## 3) HTB Summary for AQM Users

### 3.1 Core HTB mechanics

1. Tokens are consumed by packet bytes and refill over time.
2. `rate` defines guaranteed service.
3. `ceil` defines borrowable maximum when parent capacity exists.
4. Children borrow only from ancestor spare capacity.
5. Sibling contention is affected by `prio`, scheduler behavior, and class proportions.

### 3.2 Key HTB controls

- `rate`, `ceil`
- `prio`
- `burst`, `cburst`
- `quantum`, `r2q`
- `default` class minor ID (Linux HTB concept; see LibreQoS behavior note below)

### 3.3 Why this matters for `fq_codel` and `CAKE`

`fq_codel` and `CAKE` do not replace HTB's hierarchy and class-rate policy. They shape queue service inside the class envelope HTB allows.

### 3.4 Undefined traffic behavior in LibreQoS

LibreQoS behavior is explicit here:

1. traffic not mapped to a shaped circuit is passed through
2. LibreQoS does not direct undefined traffic into HTB `default` classes
3. HTB `default` class behavior still exists in Linux `tc`, but it is not how LibreQoS handles undefined traffic

Operationally, this means troubleshooting undefined traffic starts with classification/mapping validation, not with HTB default-class tuning.

### 3.5 Compact HTB skeleton (reference pattern)

Illustrative Linux HTB+leaf pattern:

```bash
tc qdisc add dev <ifname> root handle 1: htb default 30
tc class add dev <ifname> parent 1: classid 1:1 htb rate 1gbit ceil 1gbit
tc class add dev <ifname> parent 1:1 classid 1:10 htb rate 700mbit ceil 1gbit prio 1
tc class add dev <ifname> parent 1:1 classid 1:20 htb rate 300mbit ceil 1gbit prio 2
tc class add dev <ifname> parent 1:1 classid 1:30 htb rate 10mbit ceil 1gbit prio 7
tc qdisc add dev <ifname> parent 1:10 cake diffserv4
tc qdisc add dev <ifname> parent 1:20 fq_codel
tc qdisc add dev <ifname> parent 1:30 cake diffserv4
tc filter add dev <ifname> protocol ip parent 1:0 prio 1 u32 match ip src <A>/32 flowid 1:10
tc filter add dev <ifname> protocol ip parent 1:0 prio 2 u32 match ip src <B>/32 flowid 1:20
```

In LibreQoS, queue/class commands are generated automatically and undefined traffic is passed through rather than sent to an HTB default class.

## 4) fq_codel Deep Dive

### 4.1 What fq_codel is

`fq_codel` combines:

1. stochastic flow queueing (hashed queues)
2. DRR-style fair scheduling across queues
3. CoDel AQM per queue

Core references:

- `tc-fq_codel(8)`
- RFC 8290 (Flow Queue CoDel)

### 4.2 Scheduler behavior and sparse-flow benefit

FQ-CoDel maintains "new" and "old" active queue lists. Newly active queues are prioritized over persistent backlogged queues, which naturally benefits sparse/interactive traffic.

It also uses byte-credit (`quantum`) scheduling so fairness is byte-oriented, not packet-count oriented.

### 4.3 Flow hashing model

By default, packets are classified using a 5-tuple hash into a configurable number of buckets (`flows`). Hash collisions are possible and are a known tradeoff of stochastic queueing.

### 4.4 fq_codel parameters that operators actually tune

`tc-fq_codel(8)` parameters worth knowing:

1. `limit PACKETS`: hard queue packet cap (default `10240`)
2. `memory_limit BYTES`: memory cap (default `32MB`), lower of limit and memory cap is enforced
3. `flows NUMBER`: hash buckets (default `1024`, set at creation time)
4. `target TIME`: acceptable minimum persistent delay (default `5ms`)
5. `interval TIME`: CoDel control window, generally order of worst RTT through bottleneck (default `100ms`)
6. `quantum BYTES`: DRR deficit quantum (default `1514`)
7. `ecn`/`noecn`: ECN on/off (`ecn` is default in fq_codel)
8. `ce_threshold TIME`: shallow ECN marking threshold for DCTCP-style use
9. `ce_threshold_selector VALUE/MASK`: apply CE threshold only to selected traffic
10. `drop_batch`: max drop batch when limits exceed (default `64`)

### 4.5 fq_codel observability (`tc -s qdisc show`)

Common counters/fields to inspect:

1. `dropped`, `overlimits`, `requeues`
2. `drop_overlimit`
3. `new_flow_count`
4. `ecn_mark`
5. `new_flows_len`, `old_flows_len`
6. `backlog`

Interpretation pattern:

1. Verify queue pressure exists (`backlog`, `requeues`, `overlimits`)
2. Check whether AQM signaling is engaged (`ecn_mark`, `dropped`)
3. Correlate `new_flows_len`/`old_flows_len` with traffic mix (sparse vs bulk)

## 5) CAKE Deep Dive

### 5.1 CAKE architecture

CAKE integrates multiple layers in one qdisc:

1. deficit-mode shaper
2. priority queue (tins)
3. flow isolation (`DRR++`)
4. AQM (`COBALT`, combining CoDel + BLUE)
5. packet management and overhead compensation

Core references:

- `tc-cake(8)`
- Bufferbloat CAKE and CakeTechnical pages
- Piece of CAKE paper (`cake.pdf`)

### 5.2 Shaped vs unshaped operation

When `bandwidth` is set, CAKE's shaper and its derived tuning drive tin thresholds and timing behavior.

Without shaping (`unlimited`), CAKE still provides queue service and AQM logic, but tin and service behavior are no longer operating against a fixed shaped bottleneck target.

### 5.3 Flow isolation modes

CAKE supports multiple fairness modes:

1. `flowblind` (no flow isolation)
2. `flows` (5-tuple flow fairness)
3. `srchost`, `dsthost`, `hosts`
4. `dual-srchost`, `dual-dsthost`
5. `triple-isolate` (default in `tc-cake(8)`)

Operational note:

- `triple-isolate` is a safe general default when you need both per-flow and host fairness controls.

### 5.4 NAT awareness

`nat`/`nonat` controls whether CAKE performs NAT lookup before applying flow isolation.

Why it matters:

- Without `nat`, fairness sees post-NAT addresses only.
- With `nat`, fairness can better represent internal hosts behind NAT (when NAT is on the same box/path).

### 5.5 DiffServ modes and tins

Main priority presets:

1. `besteffort` (single tin, no priority queue)
2. `diffserv3`
3. `diffserv4`
4. `diffserv8`
5. `precedence` (legacy, discouraged in modern deployments)

`tc-cake(8)` currently documents `diffserv3` as default, while LibreQoS typically uses `cake diffserv4` as operator-facing default policy.

### 5.6 LibreQoS `diffserv4` DSCP mapping

LibreQoS commonly runs CAKE with `diffserv4`. Practical class mapping:

1. Latency Sensitive: `CS7`, `CS6`, `EF`, `VA`, `CS5`, `CS4`
2. Streaming Media: `AF4x`, `AF3x`, `CS3`, `AF2x`, `TOS4`, `CS2`, `TOS1`
3. Best Effort: `CS0`, `AF1x`, `TOS2`, and unrecognized codepoints
4. Background Traffic: `CS1`

Known codepoints in common operator use:

1. `CS1` (Least Effort)
2. `CS0` (Best Effort)
3. `TOS1` (Max Reliability / LLT "Lo")
4. `TOS2` (Max Throughput)
5. `TOS4` (Min Delay)
6. `TOS5` (LLT "La")
7. `AF1x`
8. `AF2x`
9. `AF3x`
10. `AF4x`
11. `CS2`
12. `CS3`
13. `CS4`
14. `CS5`
15. `CS6`
16. `CS7`
17. `VA`
18. `EF`

RFC 4594-style traffic-class framing (high-level):

1. Network Control: `CS6`, `CS7`
2. Telephony: `EF`, `VA`
3. Signaling: `CS5`
4. Multimedia Conferencing: `AF4x`
5. Realtime Interactive: `CS4`
6. Multimedia Streaming: `AF3x`
7. Broadcast Video: `CS3`
8. Low Latency Data: `AF2x`, `TOS4`
9. Ops/Admin/Management: `CS2`, `TOS1`
10. Standard Service: `CS0` and unrecognized codepoints
11. High Throughput Data: `AF1x`, `TOS2`
12. Low Priority Data: `CS1`

`fq_codel` note:

1. `fq_codel` has no CAKE tin model and no CAKE-style DSCP class scheduler.
2. DSCP marking can still be used by external classification/policy, but not via CAKE `diffserv4` tin behavior.
3. In LibreQoS, DSCP priority behavior described above applies when CAKE with `diffserv4` is selected.

### 5.7 Overhead and framing compensation

CAKE can account for link-layer overhead/framing using:

1. `overhead N`
2. `mpu N`
3. `atm`, `ptm`, `noatm`
4. shortcut keywords (`ethernet`, `docsis`, etc.)
5. `raw` and `conservative`

Operator rule:

- If overhead/framing is wrong, shaping accuracy is wrong. Validate with realistic traffic tests.

### 5.8 GSO handling (`split-gso`)

By default, CAKE splits GSO superpackets to reduce latency impact on competing flows, especially at lower rates.

At very high link rates (e.g. >10 Gbps), `no-split-gso` can improve peak throughput, but may trade away latency smoothness.

### 5.9 ACK filtering

CAKE supports:

1. `ack-filter`
2. `ack-filter-aggressive`
3. `no-ack-filter` (default)

Best use case is asymmetric links where upstream ACK load limits downstream goodput. Apply conservatively and validate with application traffic, not only synthetic tests.

### 5.10 Ingress mode and autorate

`ingress` mode changes accounting and tuning for downlink shaping realities (including counting dropped packets as already-transited data).

`autorate-ingress` can estimate capacity from arriving traffic and is primarily useful on highly variable links (for example some cellular paths). It cannot estimate bottlenecks downstream of where CAKE is attached.

### 5.11 CAKE observability (`tc -s qdisc show`)

Useful fields commonly present:

1. top-level: `dropped`, `overlimits`, `backlog`, `memory used`, `capacity estimate`
2. per tin: `thresh`, `target`, `interval`
3. delay telemetry: `pk_delay`, `av_delay`, `sp_delay`
4. hashing: `way_inds`, `way_miss`, `way_cols`
5. signaling: `drops`, `marks`
6. ACK filtering: `ack_drop`
7. queue activity: `sp_flows`, `bk_flows`, `un_flows`, `max_len`, `quantum`

Interpretation pattern:

1. check if tins and thresholds match intended policy
2. inspect delay EWMAs by tin
3. correlate `drops`/`marks` with user-visible latency and throughput
4. monitor hash-collision indicators (`way_cols`) under peak concurrency

## 6) LibreQoS Policy Framing: CAKE vs fq_codel

For LibreQoS operators, start with platform behavior first:

1. Default operation is `cake diffserv4` in HTB class leaves.
2. TreeGuard (upcoming feature) can move selected circuit directions to `fq_codel` during sustained low-load conditions and back to `cake` as utilization/guardrail pressure rises.
3. Manual per-circuit `sqm` overrides still provide explicit operator control.

Use this matrix for tradeoff context:

| Dimension | `fq_codel` | `CAKE` |
|---|---|---|
| Configuration complexity | Lower | Higher (more integrated features) |
| Resource footprint at scale | Often lower | Often higher |
| Integrated shaping features | No (needs parent shaper like HTB) | Yes (deficit-mode shaper built in) |
| DiffServ/tin behavior | Basic/indirect | Strong native tin model |
| Host isolation modes | Not CAKE-style host modes | Rich host/flow isolation modes |
| Overhead compensation | Limited | Rich built-in overhead/framing controls |
| Asymmetric-link ACK optimizations | None | ACK filtering modes available |
| Best fit | Large queue count with tight resources | Mixed traffic where policy richness and smoothness matter |

## 7) LibreQoS Operator Notes

From maintainer testing and deployment feedback:

1. `fq_codel` has no intrinsic rate limiting; it relies on HTB for rate policy.
2. `fq_codel` and `CAKE` both keep per-flow state tables, so RAM/hash behavior matters at high queue counts.
3. `CAKE` and HTB are viable down to very low and asymmetric rates in LibreQoS deployments.
4. A top-level "sandwich" limiter pattern using HTB+fq_codel is a practical deployment option in some environments.
5. Some dashboard traffic views can reflect pre-drop context; interpret counters with drop/mark semantics together.
6. "Only hard saturation benefits from AQM" is too narrow; AQM and fair queueing can improve latency when managed queues are under burst/contended pressure, even before interface-wide utilization is fully pegged.
7. Older shared/default-bucket discussion patterns reinforce that queue dynamics, not only "link fully pegged" moments, drive AQM value; in current LibreQoS, undefined traffic is pass-through, so apply this principle to managed HTB leaf queues.

## 8) Practical Observability Workflow

Start with:

```bash
tc -s qdisc show dev <ifname>
tc -s class show dev <ifname>
```

Then:

1. confirm HTB classes exist where expected
2. confirm leaf qdisc type (`cake` vs `fq_codel`) per class
3. inspect class and qdisc counters together
4. verify directionality (`ingress`/`egress`) matches the problem being diagnosed
5. correlate with user-visible latency/throughput, not counters alone

## 9) Common Misunderstandings

1. "`fq_codel` or `CAKE` replaces HTB"
   - False for LibreQoS hierarchy operation; HTB remains the policy envelope.
2. "Undefined traffic goes to an HTB default queue in LibreQoS"
   - False; LibreQoS passes undefined traffic through.
3. "Only hard saturation events benefit from AQM"
   - False. Benefits are often visible whenever a managed queue has persistent pressure (bursts, mixed-flow contention), even if total interface utilization is below 100%.
   - From maintainer testing and deployment feedback: CAKE/HTB remain useful on very low and asymmetric links, where queue control still improves usability.
   - From maintainer testing and deployment feedback: queue dynamics, not just "link fully pegged," drive AQM value; in current LibreQoS, undefined traffic is pass-through, so apply this principle to managed HTB leaves.
4. "Leaf qdisc tuning can fix broken hierarchy/class mapping"
   - False; mapping/hierarchy errors must be corrected first.
5. "`fq_codel` can rate-limit on its own"
   - False; use HTB (or another shaper) for explicit rate policy.

## 10) HTB HOWTO Context (Historical, Still Useful)

Classic HTB HOWTO material remains useful for operator mental models when translated to modern LibreQoS:

1. classify traffic
2. schedule queue service
3. shape at the bottleneck (or immediately upstream)
4. define explicit class intent with `rate`/`ceil`

Modern translation notes:

1. confirm behavior with `tc -s` counters, not assumptions about package defaults
2. keep classifier order intentional (specific rules before broader matches)
3. include explicit catch-all handling in manual `tc` deployments
4. in LibreQoS specifically, undefined traffic is pass-through unless mapped into shaped hierarchy

## 11) References

- [LibreQoS Backend Architecture](libreqos-backend-architecture.md)
- [tc-htb man page (man7)](https://man7.org/linux/man-pages/man8/tc-htb.8.html)
- [tc-fq_codel man page (man7)](https://man7.org/linux/man-pages/man8/tc-fq_codel.8.html)
- [tc-cake man page (man7)](https://man7.org/linux/man-pages/man8/tc-cake.8.html)
- [FlowQueue-Codel RFC 8290](https://www.rfc-editor.org/rfc/rfc8290)
- [IANA DSCP Registry](https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml)
- [CAKE wiki (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/Cake/)
- [CAKE technical notes (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/CakeTechnical/)
- [FQ_Codel wiki (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/FQ_Codel/)
- [CAKE vs FQ_Codel (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/Cake_vs_FQ_CODEL/)
- [CoDel/fq_codel project wiki index (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/)
- Toke Høiland-Jorgensen, Dave Taht, Jonathan Morton. *Piece of CAKE: A Comprehensive Queue Management Solution for Home Gateways*, IEEE LANMAN, 2018.
