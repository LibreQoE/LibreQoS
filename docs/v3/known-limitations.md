# Known limitations

**Version: LibreQoS v3 beta.** The operator-visible limits of the current beta.

- **Leaf queues: configured SQM, static CAKE, and dynamic allocation are
  separate.**
  - **Configured SQM.** Each circuit's queue comes from its `sqm_override`, or
    else `[queues] default_sqm` in `lqos.conf` (default `cake diffserv4`). A
    `cake…` value gives a CAKE leaf; `none` or any other value, including
    `fq_codel`, gives a FIFO leaf.
  - **Static CAKE (CakeCore).** When every circuit resolves to CAKE, every leaf
    runs CAKE all the time. This is the default.
  - **Mixed or empty trees.** If circuits resolve to different kinds, the
    bridge uses one queue for all of them, set by `LIBREQOS_DPDK_LEAF_QDISC`
    (default CoDel), and logs a warning. Per-circuit queue kinds are not yet
    supported.
  - **Dynamic CAKE allocation is off by default** (`LIBREQOS_DPDK_DYNAMIC_CAKE=1`
    to opt in). It applies only to an all-CAKE tree booted from a transaction:
    idle circuits then hold a FIFO leaf and switch to CAKE when they carry
    traffic. Otherwise the bridge keeps static CAKE and logs a warning.
- **AF_XDP is not supported.** The v3 dataplane is DPDK, on supported PCI NICs or
  virtio (`net_virtio`).
- **Worker count.** For links above 1070 Mbps, setup picks
  `workers = min(cores − 2, NIC queues, max(shaped ceiling, ceil(link / 250)))`,
  where `link` is the larger of the download and upload rates in Mbps. Core or
  queue limits can hold it below the target; setup warns when they do. Override
  it with `[queues] workers` in `lqos.conf` before first activation.
- **Changing the worker count after activation.** On a running host,
  `lqos_setup ports-plan --queues N` may fail with
  `NIC queue count undetectable`. Do not hand-edit
  `/etc/libreqos/bridge.env.d/50-ports.conf`; contact support if you need a
  different count.
- **`bridge.env` is transitional.** Settings are moving into `lqos.conf`; the
  files under `bridge.env.d/` are generated.
- **Throughput depends on the number of active circuits.** Quote throughput
  together with the circuit count and hardware.
- **Rolling back to v2 needs the v2 repository.** Keep `repo.libreqos.com` and
  `libreqos-archive-keyring.gpg` until you no longer need the rollback.
- **On-a-stick trunks** use one dataplane NIC; an inline box needs two supported
  NICs.
- **Unknown-IP tracking:** hosts stay listed for 300 s after they were last
  seen. `clear` does nothing on v3, and the rate column always reads 0 because
  unmatched traffic carries no per-flow counters.
- **NetFlow v9:** the `TCP_FLAGS` and `DST_TOS` fields are always zero, and very
  small flows may be missed.
- **Circuits above 4 Gbps split across workers:** the top-level counters in
  `bridge_leaf_stats_*.json` show one worker's share. Sum
  `worker_stats[].tx_bytes` for the circuit total.
- **IDs above 2^53** need the current WebUI; older links with such IDs do not
  resolve.
