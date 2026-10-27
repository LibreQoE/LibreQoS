# What's new in LibreQoS v3

**Version: LibreQoS v3 beta 12 (pre-release).** This page summarises the
operator-visible differences between LibreQoS v2.0 and v3. It is the honest
change list: sections that are still evolving say so.

## Beta 12

- **Auto-CAKE is off by default.** Dynamic CAKE is opt-in only
  (`LIBREQOS_DPDK_DYNAMIC_CAKE=1`); the live kind-flip path is not supported in
  beta 12.
- **`net_e1000` is packaged** for the Intel e1000/igb/igc family. No
  I210/I350/I225/I226 hardware claim is made yet; E810 is unchanged.
- **The preflight** now separates expected fresh-install notes from actionable
  warnings, treats qualified virtio as supported, and blocks an AF_XDP
  dataplane by default.
- **On-a-stick** is supported with a preflight warning; a configured inline
  pair with only one supported port is blocked.
- See [Known limitations](known-limitations.md) for the full beta list.

## The datapath changed completely

- **The kernel datapath is removed.** The v2.0 `tc`/CAKE/HTB + eBPF/XDP stack no
  longer attaches, cannot be configured, and is token-gated against reappearance.
  Do **not** follow the v2.0 pages that describe `tc`/eBPF shaping.
- **The bridge is the dataplane.** A userspace DPDK bridge (`bridge_rs`) applies
  the shaped transaction; the dataplane ports are bound to DPDK, not enslaved to
  a Linux bridge.
- **Boot is lqosd-first.** `lqosd` starts first and migrates on-disk state to the
  v3 schema; `libreqos-bridge` starts after it and **fails closed** on a v2
  transaction. The bridge keeps shaping if `lqosd` is stopped (control-plane
  independence).
- **The LKG boot** means a bridge restart or a machine reboot comes up shaping
  from the last-known-good transaction instead of waiting for a fresh control
  plan.

## The control plane

- **`lqosd`** provides the control plane, the local WebUI/dashboard, the local
  API, and the bridge broker. The retired Flask `lqos_api` is replaced by
  `libreqos-api` (fetched at install time; a failed fetch does not brick the
  install).
- **The Python refresh engine is retired.** The v3 transaction compiler produces
  the shaping world; there is no Python scheduling loop.
- **UISP** runs through `libreqos-scheduler` (the standalone UISP unit/timer is
  retired into it).

## Shaping and telemetry

- **CAKE is the default** SQM (v2.0 defaulted to `fq_codel`).
- **Dynamic (auto) CAKE allocation is OFF by default** and is **opt-in**:
  set `LIBREQOS_DPDK_DYNAMIC_CAKE=1` to enable it. When enabled, idle circuits
  hold a FIFO leaf and switch to CAKE when they carry traffic (the v2 pattern).
  **Why it is off:** the canonical F3-B install still asserts a kind change on
  existing circuits while the armer owns the materialized kind, so an ON world
  can be refused by the worker's live-kind guard (`docs/74t143_armA_root_cause.md`).
  The normal-lane kind ownership (`lqos-74t.144`) must land before this is
  re-enabled by default. With auto CAKE OFF, every circuit materializes its
  configured AQM (the CAKE default); the `.143` CAKE-lane isolation work is not
  part of this release.
  The **aggregate sub-gate** (`LIBREQOS_DPDK_DYNAMIC_CAKE_AGGREGATE_GATE`) is
  default-off and inert without shard-tin. The load-bearing number is the
  **concurrent-CAKE ceiling**, not the subscriber count, so this is **not** a
  delivered-scale claim.
- **Per-direction RTT / retransmit / packet counters are back** end-to-end
  (flows, hosts, the heatmap percentiles, the live views), including the pooled
  QoO rollup returning to kernel semantics.
- **Unknown-IP tracking is implemented.** The page and `#unknownIpCount` render
  the bridge **best-effort (unmapped) host** class. Three honest notes: the page
  keeps a 300 s last-seen window; `clear` on a bridge host is a disclosed no-op
  (the source is a live interval snapshot, not an accumulating kernel map); and
  the unmapped class carries no flow deltas, so its rate reads **0 by design**
  (the page shows the host and a real Last Seen).
- **Heatmap accounting** changed from median to exact per-minute accounting; the
  site/global denominators are the summed member capacity.

## Configuration surface

The v3 config surface is smaller and stricter than v2.0's:

- **`/etc/libreqos/lqos.conf`** — the product network/shaping world. The v2.0
  compat pair (`network.json` + `ShapedDevices.csv`) is still read for migration.
- **`/etc/libreqos/bridge.env`** — the bridge's environment (typed config path,
  EAL args, the analytics socket, the LKG path). Operators edit this file, never
  the unit.
- **`/etc/libreqos/bridge.conf`** — the typed JSON runtime config
  (`schema_version`). Strict: an unknown field is rejected, so a stale v2.0 key
  **blocks the bridge from starting**. Setup migrates the known legacy key
  (`circuit_capacity` is removed automatically); see
  [Upgrading from v2.0](upgrade-from-v2.md).
- **Removed / retired keys**: `fast_queues_fq_codel`, `queue_check_period_ms`,
  `circuit_fallback_persist`, the dead `[treeguard]` template block, the dead
  `runtime_virtualized` UI, and the static node-virtualization surface
  (`[topology] queue_auto_virtualize_threshold_mbps`, the per-node
  `queue_visibility_policy`, and the Network Map "Virtual" toggle).
  `[tuning]` netdev/ethtool keys are relabelled
  "Management NIC / Kernel Netdev only" — they do **not** tune DPDK ports.
- **Virtual nodes are gone.** v2 virtual-node groupings model as **real sites**
  in the source (`network.json`); a `virtual` flag is ignored. The bridge handles
  tree depth natively and site sharding distributes subtrees across workers, so a
  node is never hidden or have its children promoted at runtime — every canonical
  node shapes as a real queue. (The `SetNodeVirtual` override and the TreeGuard
  runtime-virtualization layer were already retired with TreeGuard.)
- **`do_not_track_subnets`** is restored: a per-subnet opt-out that suppresses
  flows **and** RTT for matching IPs (and is honoured by hosted Insight).
- **UISP reporting interval**: the v2.0 `uisp_reporting_interval_seconds`
  migrates into `integration_common.queue_refresh_interval_mins`
  (when the live value is still the default).

## Honest caveats (do not misquote)

- Throughput walls scale with the **active leaf count** — never cite a blanket
  "40G" figure without the leaf count and hardware qualifiers.
- The bufferbloat result is a **circuit-level** claim (plus the in-situ
  evaluation); system-level numbers depend on the deployment.
- The NetFlow v9 exporter emits `TCP_FLAGS` / `DST_TOS` as honest zeros (the
  bridge has no wire source), and small-flow attribution is bounded by the
  tracked-table property quantified in the release notes.
- **Sharded (W>1) per-leaf counters:** for a circuit sharded across workers
  (a >4 Gbps circuit on a multi-worker shard-tin deployment), the top-level
  `bridge_leaf_stats_*.json` counters are **one worker's share**, not the circuit
  aggregate — read the circuit's egress as **Σ `worker_stats[].tx_bytes`** (the NIC
  `ipackets` is the total cross-check; plus iperf3 and the shard-tin snapshot).
  Non-sharded circuits are unaffected.

## Insight and APIs

Insight and the local API are v3-native. 64-bit ids require the string-carrying
WebUI; links composed before that fix dead-end for ids above 2^53.
