# Troubleshooting LibreQoS v3

**Version: LibreQoS v3.** Common v3 failures and where to look. The v2.0
troubleshooting steps do not apply to v3.

## First: collect the support bundle

```bash
sudo libreqos-support-bundle /tmp
```

Attach the resulting tarball to a support ticket. To read the logs yourself:

```bash
journalctl -u libreqos-bridge -n 200 --no-pager
journalctl -u libreqos-lqosd   -n 200 --no-pager
journalctl -u libreqos-setup   -n 200 --no-pager
```

## The install is blocked

- **Preflight BLOCK**: the host is unsupported (old OS, unsupported NIC pair, or
  your network config does not compile under v3). The output names the reason.
  Blocked hosts stay on v2.
- **Hugepages**: if the 2M page reservation failed, add
  `default_hugepagesz=2M hugepages=3072` to the kernel command line and reboot.
  The setup check warns about missing pages but does not block.

## The bridge refuses to start

- **"invalid runtime config … unknown field"**: `/etc/libreqos/bridge.conf`
  has a key v3 does not know. Setup removes the old `circuit_capacity` key
  automatically; if you edited the file by hand, remove the key the error
  names.
- **Port check fails**: a dataplane NIC still has an IP address, a
  bridge/bond/VLAN/macvlan on top of it, or is still in a netplan bridge. See
  the adoption step in [Upgrading from v2.0](upgrade-from-v2.md).
- **Anything else**: collect the support bundle and open a ticket.

## The dashboard is blank while traffic flows

`lqosd` is not receiving the bridge's statistics. Check that
`LIBREQOS_ANALYTICS_SOCKET` is set for both `libreqos-bridge` and
`libreqos-lqosd`. If the page itself fails to load, reinstall the `libreqos`
package to restore the WebUI files.

## The control channel drops when someone logs out

systemd's `RemoveIPC` deletes the control channel's shared memory when the last
login session of its owning user ends. The `libreqos-dataplane` package installs
`/etc/systemd/logind.conf.d/50-libreqos-ipc.conf` to prevent this. Check that the
file exists (restore it by reinstalling `libreqos-dataplane`, then restart
`systemd-logind`), and do not log in over SSH as the LibreQoS service user.

## Shaping did not come back after a reboot

The bridge restarts from `/var/lib/libreqos-bridge/bridge_lkg.json`. If that file
is missing or corrupt, the bridge can stop with a `FATAL` on its first boot
instead of shaping. Check that `LIBREQOS_LKG_FILE` in `bridge.env` points to
`/var/lib/libreqos-bridge/` and not to `/run`, which is cleared at boot.
