# Operating LibreQoS v3

**Version: LibreQoS v3 beta 12 (pre-release).** How to check, change, and
support a running v3 host. See [Known limitations](known-limitations.md) for the
beta scope.

## The units

| Unit | Role | Starts |
|---|---|---|
| `libreqos-setup` | oneshot verify + on-disk migration | first |
| `libreqos-lqosd` | control plane, local API, WebUI, bridge broker | after setup |
| `libreqos-bridge` | the DPDK dataplane (shaping) | after lqosd (`Requires=` setup) |
| `libreqos-scheduler` | integrations hub (incl. UISP) | after lqosd |
| `libreqos-power` | power-profile helper | on demand |
| `libreqos-api` | the local HTTP API (enabled on a successful install-time fetch) | at install |

The `libreqos` control-plane package provides setup/lqosd/scheduler/power/api;
**`libreqos-bridge` (and its binary + DPDK runtime) comes from the separate
`libreqos-dataplane` artifact** (`update_bridge.sh`; see
[Installing v3](install.md)).

```bash
systemctl status libreqos-setup libreqos-lqosd libreqos-bridge libreqos-scheduler libreqos-power libreqos-api
```

**Control-plane independence:** stopping `lqosd` does **not** stop traffic
shaping — the bridge keeps applying the last committed world. A bridge crash is
restarted by systemd and boots from the LKG.

## Seam health

The bridge↔lqosd seam can be read directly:

```bash
lqosd --print-seam-state
cat /run/libreqos/seam_state.json
```

A healthy seam means the bridge is attached and the control channel/analytics
socket are live. `lqosd` never gains privilege beyond the three sudoers verbs.

## Configuration you may edit

- **`/etc/libreqos/lqos.conf`** — the product network/shaping world (integrations,
  queues, `[dataplane]` bandwidth, `[flows] do_not_track_subnets`, license).
- **`/etc/libreqos/bridge.env`** — the bridge environment. The production wiring
  keys (`LIBREQOS_DPDK_USE_LPM_CLASSIFIER`,
  `LIBREQOS_DPDK_TRANSACTION_FILE`, `LIBREQOS_ANALYTICS_SOCKET`, the LKG path)
  live here. **Operators edit this file, never the systemd unit.**
- **`/etc/libreqos/bridge.conf`** — typed JSON runtime config (`schema_version`).
  Strict: an unknown field is rejected.

Changing the **queue mode** (`shape` ↔ `observe`) is a **supervised restart** of
`libreqos-bridge`, never a runtime toggle.

## WebUI and API

`lqosd` serves the operator dashboard and the local API. If the dashboard is dark
while traffic flows, the analytics socket is missing or the web UI bundles were
not installed (see `packaging/INSTALL.md`). The tree view is served over a
WebSocket (not HTTP).

## Support bundle

Collect everything a support ticket needs with the packaged, read-only tool:

```bash
sudo libreqos-support-bundle [outdir]
```

It gathers systemd/journal state, the bridge log + leafstats, LKG metadata, the
env/config, hugepages, links, `lspci`, and `dpdk-devbind`. It never restarts
anything.

## Deploy rule: the full matched set

A `lqosd`/control change is the **control** package; a `bridge_rs` change is the
**dataplane** package. Install the **full matched set** together, stop the bridge
first, then `dpkg --configure -a`. Mixing a control-only candidate with an older
dataplane is the silent-teardown class.
