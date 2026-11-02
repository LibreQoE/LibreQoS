# Known limitations

**Version: LibreQoS v3 beta (current pre-release).** This page lists the
operator-visible limits of the current beta. It is updated with the package
facts, not aspirational features.

- **Auto CAKE is off by default.** FIFO/CoDel is the shipped leaf AQM; CAKE is
  opt-in. Do not assume CAKE semantics from v2.0.
- **AF_XDP is not the v3 dataplane.** The supported dataplane is DPDK (PCI
  PMDs) or the packaged virtio `net_virtio` path. The retired AF_XDP posture
  is v2.0-only.
- **Worker autosizing is CPU/queue bounded.** For a link above 1070 Mbps the
  setup derives `workers = min(cores-2, queues, max(shaped_ceiling,
  ceil(link/250)))` where `link = max(down, up)`. A hard Cores/Queues cap can
  still bind below the target; the managed plan carries the warning. The
  operator override is `[queues] workers` in `lqos.conf`.
- **Plan-writer bound-NIC bug (beta16).** `lqos_setup ports-plan --queues N`
  may still report `NIC queue count undetectable` on an already-bound NIC.
  Tracked in `lqos-74t.89`; do not hand-edit `50-ports.conf`.
- **`bridge.env`/`bridge.env.d` are transitional.** The operator source is
  `/etc/libreqos/lqos.conf`; the generated plan is machine-written. The
  bridge.env retirement is staged in `lqos-74t.89`.
- **A v2.0 rollback needs the legacy source/key.** Keep
  `repo.libreqos.com` and `libreqos-archive-keyring.gpg` until the rollback
  window closes; `libreqos-v3-rollback --from-archive` uses the legacy
  `archive` suite.
- **On-a-stick trunks** use one dataplane NIC (WAN == LAN BDF); a two-port
  inline box needs two supported NICs.
