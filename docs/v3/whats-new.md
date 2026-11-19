# What's new in LibreQoS v3

**Version: LibreQoS v3 beta.** The operator-visible changes from v2.0.

## The datapath

- **The kernel datapath is removed.** The v2.0 `tc`/CAKE/HTB and eBPF/XDP stack
  is gone. A userspace DPDK bridge (`bridge_rs`) does the shaping, and the
  dataplane NICs are bound to DPDK instead of a Linux bridge.
- **`lqosd` starts first.** It upgrades the on-disk state to the v3 format, then
  the bridge starts. The bridge refuses to start on leftover v2 state rather
  than shaping wrongly.
- **Shaping survives a control-plane outage.** If `lqosd` stops, the bridge
  keeps shaping with the last applied configuration.
- **Restarts come up shaping.** After a bridge restart or a reboot, the bridge
  shapes immediately from its last-known-good (LKG) configuration instead of
  waiting for `lqosd`.

## The control plane

- **`lqosd`** serves the WebUI, the node API at `/api/v3`, and the link to the
  bridge. The separate Flask `lqos_api` service is retired.
- **The Python refresh engine is retired.** A built-in compiler builds the
  shaping configuration.
- **UISP** runs inside `libreqos-scheduler`; the separate UISP unit and timer
  are retired.

## Shaping and telemetry

- **Your SQM setting still applies.** `[queues] default_sqm` (default
  `cake diffserv4`, as in v2.0) and per-circuit `sqm_override` pick each leaf
  queue. CAKE runs as the bridge's own CakeCore. `fq_codel` has no v3
  implementation, and such circuits get a FIFO leaf. See
  [Known limitations](known-limitations.md).
- **Dynamic CAKE allocation is available but off by default.** With
  `LIBREQOS_DPDK_DYNAMIC_CAKE=1` on an all-CAKE tree, idle circuits run a
  simple FIFO queue and switch to CAKE when they carry traffic. Capacity then
  depends on how many circuits run CAKE at once, not on the total subscriber
  count.
- **Per-direction RTT, retransmit and packet counters are back** in flows, hosts,
  the heatmap and the live views.
- **Unknown-IP tracking** shows traffic from hosts that match no circuit. See
  [Known limitations](known-limitations.md) for how it differs from v2.
- **The heatmap** uses exact per-minute values instead of medians. Site and
  global percentages are relative to the summed capacity of their members.

## Configuration

- **`/etc/libreqos/lqos.conf`** is the main config. v2's `network.json` and
  `ShapedDevices.csv` are still read.
- **`/etc/libreqos/bridge.env`** and **`/etc/libreqos/bridge.conf`** hold bridge
  settings; see [Operating v3](operate.md).
- **Retired keys** are ignored: `fast_queues_fq_codel`, `queue_check_period_ms`,
  `circuit_fallback_persist`, the `[treeguard]` block, and the node-virtualization
  settings (`queue_auto_virtualize_threshold_mbps`, `queue_visibility_policy`,
  and the Network Map "Virtual" toggle). The `[tuning]` keys apply only to the
  management NIC, not the DPDK ports.
- **Virtual nodes are gone.** A node marked `virtual` in `network.json` is
  shaped as a real site. The bridge handles deep trees natively.
- **`do_not_track_subnets`** is back: matching IPs are left out of flow and RTT
  tracking, including in Insight.
- **The UISP reporting interval** carries over from v2 automatically.

## Insight and APIs

Insight and the local API are v3-native.
