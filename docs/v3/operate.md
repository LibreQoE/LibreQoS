# Operating LibreQoS v3

**Version: LibreQoS v3 beta.** How to check, change and support a running v3
host.

## The services

| Unit | Role | Starts |
|---|---|---|
| `libreqos-setup` | one-shot hugepage/NIC check | first |
| `libreqos-lqosd` | control plane, local API, WebUI | after setup |
| `libreqos-bridge` | DPDK dataplane (shaping) | after lqosd |
| `libreqos-scheduler` | integrations (including UISP) | after lqosd |
| `libreqos-power` | power-profile helper | on demand |

The `libreqos` package provides all of these except `libreqos-bridge`, which
comes from `libreqos-dataplane`. The node API at `/api/v3` is part of `lqosd`.

```bash
systemctl status libreqos-setup libreqos-lqosd libreqos-bridge libreqos-scheduler libreqos-power
```

Stopping `lqosd` does not stop shaping: the bridge keeps the last applied
configuration. If the bridge crashes, systemd restarts it and it comes back
shaping from its last-known-good file.

## Bridge connection health

```bash
lqosd --print-seam-state
```

A healthy result shows the bridge attached and the control and analytics
channels live. The same data is in `/run/libreqos/seam_state.json`.

## Files you may edit

- **`/etc/libreqos/lqos.conf`**: the main config (integrations, queues,
  `[dataplane]` rates, `[flows] do_not_track_subnets`, license).
- **`/etc/libreqos/bridge.env`**: the bridge's environment. Edit this file,
  never the systemd unit.
- **`/etc/libreqos/bridge.conf`**: typed JSON bridge config. An unknown key
  stops the bridge from starting.

Do not edit the generated files under `/etc/libreqos/bridge.env.d/`.

Switching the queue mode between `shape` and `observe` takes a restart of
`libreqos-bridge`.

## WebUI and API

`lqosd` serves the dashboard on port 9123 and the local API. The dashboard frame
can be shown in English or Spanish; see [Web UI language](node-manager-i18n.md).
If the dashboard is blank while traffic flows, see
[Troubleshooting](troubleshooting.md).

## Support bundle

```bash
sudo libreqos-support-bundle [outdir]
```

This collects service and journal state, the bridge log and stats, the LKG
metadata, the configs, hugepages, links, `lspci` and `dpdk-devbind` output. It
is read-only and restarts nothing.

## Updating

Always install `libreqos`, `libreqos-preflight` and `libreqos-dataplane` at the
same version, through apt; see the
{ref}`beta-to-beta upgrade <v3-beta-upgrade>`. Mixing a new control
plane with an older dataplane can stop shaping.
