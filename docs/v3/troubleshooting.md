# Troubleshooting LibreQoS v3

**Version: LibreQoS v3 (current).** Common v3 failures and where to look. For a
full v2.0 kernel-datapath troubleshooting page, see the
[v2.0 archive](../v2.0/index.md) — those steps do **not** apply to v3.

## First: collect the support bundle

```bash
sudo libreqos-support-bundle /tmp
```

Attach the resulting tarball to a support ticket. It includes systemd/journal
state, the bridge log + leafstats, the LKG metadata, the config/env, hugepages,
links, `lspci`, and `dpdk-devbind`. Live logs are also visible per unit:

```bash
journalctl -u libreqos-bridge -n 200 --no-pager
journalctl -u libreqos-lqosd   -n 200 --no-pager
journalctl -u libreqos-setup   -n 200 --no-pager
```

## The bridge refuses to start

- **"invalid runtime config … unknown field"** — `/etc/libreqos/bridge.conf` has
  a key v3 does not know (commonly the legacy `circuit_capacity`). Setup removes
  the known legacy key automatically; if you hand-edited the file, remove the
  offending key. The parser is intentionally strict (fail closed), so a typo is
  loud, never silently ignored.
- **"circuit capacity read before seeding"** — a v3-internal seed-ordering fault,
  not an operator error. Collect the bundle and raise it.
- **Port preflight fails** — a dataplane NIC still has a global address, or a
  bridge/bond/VLAN/macvlan upper, or is still enslaved by a netplan bridge. See
  the adoption step in [Upgrading from v2.0](upgrade-from-v2.md).

## The install is blocked

- **Preflight BLOCK** — the host is unsupported (old OS/glibc, unsupported NIC
  pair, or the config does not compile). The verdict names the reason. Blocked
  hosts stay on v2.x, which keeps maintenance for them.
- **Hugepages** — if the runtime 2M allocation failed, add
  `default_hugepagesz=2M hugepages=3072` to the kernel command line and reboot,
  or use the 1G posture. The setup check warns (never blocks) on missing pages.

## The dashboard is dark while traffic flows

The analytics socket is missing or the web UI bundles are not installed. Confirm
`LIBREQOS_ANALYTICS_SOCKET` reaches **both** `libreqos-bridge` and
`libreqos-lqosd`, and that the `static2` bundles were installed (see
`packaging/INSTALL.md`).

## The control channel is blacked out

Systemd's `RemoveIPC` deletes the libreqos-owned control segment when the last
login session of its owner ends. Install
`logind/50-libreqos-ipc.conf` to `/etc/systemd/logind.conf.d/` and restart
`systemd-logind`. Operators should not ssh as the service user on a bridge host.

## After a reboot, shaping did not come back

The bridge boots from the last-known-good transaction. If
`/var/lib/libreqos-bridge/bridge_lkg.json` is absent or corrupt, you can hit the
documented first-boot FATAL instead of shaping. Confirm the LKG path in
`bridge.env` points at persistent storage — not the tmpfs default under `/run`.
