# Upgrading from LibreQoS v2.0 to v3

**Version: LibreQoS v3 beta.** Do not upgrade a production v2 host in place
without a way back. Snapshot or back up the box first, have console access if
management traffic crosses the bridge, and avoid doing this over a flaky remote
link: the path includes a reboot.

v3 takes the two dataplane NICs for DPDK instead of putting them in a Linux
bridge. The v2 bridge must therefore be removed from netplan ("adopted") before
the ports are bound.

## 1. Add the v3 repository, keep the v2 one

The v2 repository (`repo.libreqos.com`) does not carry v3. Add the v3 repository:

```bash
curl -fsSL https://packages.libreqos.com/keys/libreqos-packages-keyring.gpg \
  | sudo tee /usr/share/keyrings/libreqos-packages-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/libreqos-packages-keyring.gpg] https://packages.libreqos.com beta main" \
  | sudo tee /etc/apt/sources.list.d/libreqos.list
sudo apt-get update
```

If `libreqos.list` pointed at the v2 repository, this replaces it. Other v2
source files are disabled by the v3 package on install (renamed with a
`.v2-disabled` suffix), not deleted. **Keep
`/usr/share/keyrings/libreqos-archive-keyring.gpg`**: the rollback installs v2
from the v2 archive and needs it.

## 2. Run the preflight check

```bash
sudo apt-get install -y libreqos-preflight && libreqos-v3-preflight
```

It reports PASS / WARN / BLOCK for the hardware, whether your existing
`network.json` and `ShapedDevices.csv` compile under v3, the license count, and
the current install. **A BLOCK means stop here**; the host stays on v2.

Watch for these lines:

- `netplan: bridge br0 over ... (adoption required)`: step 4 is needed.
- `OPERATOR-CONSOLE REQUIRED`: management appears to use the bridge. Do not
  continue without console access.

## 3. Install v3

```bash
sudo apt-get install -y libreqos libreqos-dataplane
```

Installing `libreqos` stops the v2 daemons. If the install is refused on a
BLOCK, restart the v2 daemons and fix the block before retrying. Your v2
configs are read in place, never rewritten.

## 4. Adopt the Linux bridge

Run the network-mode adoption in `sudo lqos_setup tui` (or the WebUI). It
removes the bridge from netplan as a reversible change. **Write down the backup
id it prints**; the rollback needs it. If you lose it, list the stored backups
(newest in `last_backup_id`, the others in `recent_backups`):

```bash
sudo /opt/libreqos/src/bin/lqos_netplan_helper status
```

Then check that management still answers before you go on.

## 5. Finish setup

Continue in `lqos_setup tui`: pick the NIC pair and activate. The services then
start, and `libreqos-bridge-bind.service` binds the dataplane NICs to DPDK. The
bind is refused while a NIC is still in a netplan bridge; if you see that error,
the adoption did not take, or another netplan file still owns the NIC.

## 6. Check the migrated configuration

Most of this happens automatically:

- **Rates** come from `lqos.conf [dataplane] download_mbps` / `upload_mbps`.
  The v2 keys `[queues] downlink_bandwidth_mbps` / `uplink_bandwidth_mbps` still
  work as a fallback, and setup copies them into `[dataplane]` (backup:
  `.pre-s7.bak`).
- **`bridge.conf`** is strict: an unknown key stops the bridge. Setup removes the
  old `circuit_capacity` key. A hand-edited file is left alone; a `bandwidth`
  block in it now stops the bridge, so remove it and set the rates in
  `lqos.conf`.
- **Retired v2 keys** are ignored; see [What's new](whats-new.md).
- **The UISP interval** (`uisp_reporting_interval_seconds`) becomes
  `integration_common.queue_refresh_interval_mins` unless you already set the
  latter.

## 7. Verify

Use the {ref}`install checks <v3-verify>`. Also confirm `lqosd --print-seam-state`
shows the bridge attached, and that traffic is shaped.

## 8. Roll back if needed

1. Restore the netplan adoption backup (its id is from step 4, or from
   `lqos_netplan_helper status`) so the bridge is back. This restores the saved
   netplan files and runs `netplan apply`, so use the console:

   ```bash
   sudo /opt/libreqos/src/bin/lqos_netplan_helper rollback <backup-id>
   ```
2. Run the rollback, using a saved v2 `.deb` or the v2 archive:

   ```bash
   sudo libreqos-v3-rollback --deb <path-to-v2.deb>
   # or
   sudo libreqos-v3-rollback --from-archive <v2-version>
   ```

   It stops and removes v3 and installs v2. Your v2 configs are untouched.
3. Reboot.
