# Installing LibreQoS v3

LibreQoS v3 is the DPDK dataplane plus the `lqosd` control plane. This page has
three paths; use only the one that matches your host.

- **Fresh v3 install** — a new Ubuntu 24.04 host.
- **Beta-to-beta upgrade** — an existing v3 beta host.
- **v2.0 to v3** — a production v2 host; staged, see
  [Upgrading from v2.0](upgrade-from-v2.md).

## Before you start

- Ubuntu Server 24.04 LTS or newer, root/sudo.
- Two supported dataplane NICs (or one for an on-a-stick trunk). The dataplane
  NICs must have no management address and no bridge/bond/VLAN upper.
- Hugepages: the installer tries 3072 × 2M at install. If that fails, add
  `default_hugepagesz=2M hugepages=3072` to the kernel command line and reboot.
- **Installation is offline.** Python dependencies ship in a vendored wheelhouse
  inside the `libreqos` package; the postinst installs them with
  `pip --no-index` and does not contact PyPI.
- Proxmox/VM: use the modern virtio device `1af4:1041` and
  `disable-legacy=on` on both dataplane NICs. See
  [Upgrading from v2.0](upgrade-from-v2.md) for the full host/NIC detail.

## Fresh v3 install

```bash
# 1. Add the LibreQoS v3 apt repo (beta channel).
curl -fsSL https://packages.libreqos.com/keys/libreqos-packages-keyring.gpg \
  | sudo tee /usr/share/keyrings/libreqos-packages-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/libreqos-packages-keyring.gpg] https://packages.libreqos.com beta main" \
  | sudo tee /etc/apt/sources.list.d/libreqos.list
sudo apt-get update

# 2. Check the host before installing anything.
sudo apt-get install -y libreqos-preflight
libreqos-v3-preflight        # PASS / WARN / BLOCK

# 3. Install the three packages at ONE version. The node API is served by
#    lqosd at /api/v3; there is no separate libreqos-api package or unit.
V=$(apt-cache policy libreqos | awk '/Candidate:/ {print $2}')
case "$V" in ""|"(none)") echo "no libreqos candidate"; exit 1;; esac
echo "installing $V"
sudo apt-get install -y \
  libreqos-preflight="$V" libreqos="$V" \
  libreqos-dataplane="$V"

# 4. Reinstall trap: v3 reads /etc/libreqos/lqos.conf. A stale v2-layout
#    /etc/lqos.conf is NOT the v3 config; move it aside.
if [ -f /etc/lqos.conf ]; then
  echo "WARNING: legacy /etc/lqos.conf exists; v3 uses /etc/libreqos/lqos.conf"
fi

# 5. First run (NIC pair, rates, users, then activate).
sudo lqos_setup tui
```

**Worker count before first activation.** If this box needs a fixed worker
count, set it **before** the TUI's final `activate-runtime` step (the activation
is what renders the plan and starts the bridge). At the final confirmation
prompt, open a second terminal and edit `/etc/libreqos/lqos.conf`:

```toml
[queues]
workers = 8
```

Then confirm activation. The TUI preserves the key and its final step renders
the plan at that W.

If the TUI did not start the units (or you exited before activation), start
them **in order** and wait for each:

```bash
wait_active() { # wait_active UNIT TIMEOUT_SEC
  local i=0
  while [ "$i" -lt "$2" ]; do
    systemctl is-active --quiet "$1" && return 0
    sleep 2; i=$((i+2))
  done
  echo "TIMEOUT waiting for $1"; return 1
}
sudo systemctl start libreqos-lqosd && wait_active libreqos-lqosd 60
sudo systemctl start libreqos-scheduler && wait_active libreqos-scheduler 60
sudo systemctl start libreqos-bridge && wait_active libreqos-bridge 120
```

The bearer-authenticated node API is served by `lqosd` on the WebUI listener
under `/api/v3` (see the [Node API](../v2.0/api.md)); there is no separate
`libreqos-api` unit to start, and no `:9122` listener.

Set the rates and any worker override in `/etc/libreqos/lqos.conf` (the single
operator source):

```toml
[dataplane]
download_mbps = 2500.0
upload_mbps = 2500.0

[queues]
workers = 8          # optional; omit to autosize
```

`workers` is the DPDK worker count. Omit it and the installer derives a count
from the link, physical cores, and NIC queues. If the box is CPU-limited at the
derived count, set `[queues] workers` and re-render the plan. Read the bound BDF
pair from the machine state first:

```bash
# The STATE reader is FIRST-wins (bootstrap::state_value), so take head -1.
WAN_BDF=$(sed -n 's/^LQOS_DPDK_ORIG_WAN_BDF=//p' \
  /var/lib/libreqos-bridge/dpdk-bind-originals.env 2>/dev/null | head -1)
LAN_BDF=$(sed -n 's/^LQOS_DPDK_ORIG_LAN_BDF=//p' \
  /var/lib/libreqos-bridge/dpdk-bind-originals.env 2>/dev/null | head -1)
# mlx5 in-place boxes may have no bind-originals STATE: fall back to the
# managed plan's BDF keys, then to lspci (no hard exit).
if [ -z "$WAN_BDF" ] || [ -z "$LAN_BDF" ]; then
  WAN_BDF=$(sed -n 's/^LQOS_DPDK_WAN_BDF=//p' \
    /etc/libreqos/bridge.env.d/50-ports.conf 2>/dev/null | head -1)
  LAN_BDF=$(sed -n 's/^LQOS_DPDK_LAN_BDF=//p' \
    /etc/libreqos/bridge.env.d/50-ports.conf 2>/dev/null | head -1)
fi
if [ -n "$WAN_BDF" ] && [ -n "$LAN_BDF" ]; then
  # beta16: pass the BDF pair explicitly
  sudo lqos_setup ports-plan --wan "$WAN_BDF" --lan "$LAN_BDF"
  # once lqos-74t.89 lands, the config-only form is:
  # sudo lqos_setup ports-plan --from-config
  sudo systemctl restart libreqos-bridge
else
  echo "BDF pair not found in STATE/50-ports; identify the two dataplane NICs:"
  lspci -nn | grep -Ei 'ethernet|network'
  echo "set WAN_BDF=... LAN_BDF=... and rerun the ports-plan command"
fi
```

`--wan`/`--lan` supply only the PCI pair; the queue count still comes from the
NIC/STATE. `--queues N` is accepted, but on beta16 it may still report the
bound-NIC error below (74t.89). Expected:
`ports-plan: wrote /etc/libreqos/bridge.env.d/50-ports.conf — workers=8 ...`.
`ports-plan` writes that file; do not hand-edit `bridge.env`, `bridge.env.d/*`,
or `50-ports.conf`. `libreqos-setup.service` verifies hugepages/NICs; it does
not write the plan.

Verify (all three at one version, no restarts):

```bash
dpkg-query -W -f='${Package} ${Version}\n' \
  libreqos libreqos-preflight libreqos-dataplane
systemctl is-active libreqos-lqosd libreqos-scheduler libreqos-bridge
for u in libreqos-lqosd libreqos-scheduler libreqos-bridge; do
  echo "$u NRestarts=$(systemctl show -p NRestarts --value "$u")"
done
journalctl -u libreqos-bridge --since "-10 min" | grep -E "BOOT-PLACEMENT|FATAL"
```

Expect the same `V` on all three lines, `active` on all three units, and no
`FATAL`. **First activation may recycle the bridge** (`NRestarts=2` is not a
failure if the unit is active now). If the bridge is `failed`, reset and
restart it once, then re-check:

```bash
sudo systemctl reset-failed libreqos-bridge 2>/dev/null || true
sudo systemctl restart libreqos-bridge
wait_active() { local i=0; while [ "$i" -lt 120 ]; do
  systemctl is-active --quiet libreqos-bridge && return 0
  sleep 2; i=$((i+2)); done; return 1; }
wait_active || { echo "bridge still not active"; exit 1; }
systemctl show -p NRestarts --value libreqos-bridge
```

## Beta-to-beta upgrade

Back up the config, the LKG, and the old packages **before** changing
anything. The three `$OLD` debs go into `$BK` (the rollback block below uses
that same directory):

```bash
OLD=$(dpkg-query -W -f='${Version}' libreqos 2>/dev/null || true)
test -n "$OLD" || { echo "libreqos is not installed; this is not an upgrade"; exit 1; }
BK=/var/backups/libreqos-upgrade-$(date +%Y%m%dT%H%M%S)
sudo install -d -m 0700 "$BK"
sudo cp -a /etc/libreqos "$BK/etc-libreqos"
test -f /var/lib/libreqos-bridge/bridge_lkg.json \
  && sudo cp -a /var/lib/libreqos-bridge/bridge_lkg.json "$BK/bridge_lkg.json"
# REQUIRED: capture the three old debs now. apt beta normally serves only the
# current candidate, so this can fail; if it does, STOP and obtain the three
# $OLD debs from your own snapshot. Do not upgrade without a tested rollback.
( cd "$BK" && sudo apt-get download \
    libreqos="$OLD" libreqos-preflight="$OLD" \
    libreqos-dataplane="$OLD" ) || {
  echo "FAIL: could not download the $OLD debs; obtain them from your snapshot before upgrading" >&2
  exit 1
}
for p in libreqos-preflight libreqos libreqos-dataplane; do
  ls "$BK"/${p}_*.deb >/dev/null 2>&1 || {
    echo "FAIL: backup is missing $p=$OLD; do not upgrade" >&2
    exit 1
  }
done
echo "$OLD" | sudo tee "$BK/old-version" >/dev/null
echo "backup: $BK  old version: $OLD (three debs captured)"
```

Install the three packages at one version:

```bash
sudo apt-get update
V=$(apt-cache policy libreqos | awk '/Candidate:/ {print $2}')
case "$V" in ""|"(none)") echo "no libreqos candidate"; exit 1;; esac
echo "upgrading $OLD -> $V"
sudo apt-get install -y -o Dpkg::Options::=--force-confold \
  libreqos-preflight="$V" libreqos="$V" \
  libreqos-dataplane="$V"
```

Restart in order, waiting on each unit (no fixed sleeps):

```bash
wait_active() { # wait_active UNIT TIMEOUT_SEC
  local i=0
  while [ "$i" -lt "$2" ]; do
    systemctl is-active --quiet "$1" && return 0
    sleep 2; i=$((i+2))
  done
  echo "TIMEOUT waiting for $1"; return 1
}
sudo systemctl daemon-reload
sudo systemctl restart libreqos-lqosd && wait_active libreqos-lqosd 60
sudo systemctl restart libreqos-scheduler && wait_active libreqos-scheduler 60
sudo systemctl restart libreqos-bridge
# The bridge's own boot gate waits up to 120 s for a real transaction/boot
# source; do not replace this with a sleep.
wait_active libreqos-bridge 120
```

Verify with the same block as the fresh path. On the first boot after an
upgrade, the expected placement lines are:

```text
BOOT-PLACEMENT ... path=tx ... section_validation=rejected:placement_budget
Placement ownership ACTIVE ...
```

That is the A0 re-place, not a failure. A `FATAL` line is a failure.

**Rollback** (restore the backup, then the old three-package set, then restart
in the same order):

```bash
# Self-contained rollback: discover the newest backup and its old version if
# this is a fresh shell (the upgrade block's BK/OLD are not required).
BK=${BK:-$(ls -dt /var/backups/libreqos-upgrade-* 2>/dev/null | head -1)}
[ -n "$BK" ] && [ -d "$BK" ] || { echo "backup dir not found"; exit 1; }
OLD=${OLD:-$(cat "$BK/old-version" 2>/dev/null || true)}
wait_active() { # self-contained: paste with this block
  local i=0
  while [ "$i" -lt "$2" ]; do
    systemctl is-active --quiet "$1" && return 0
    sleep 2; i=$((i+2))
  done
  echo "TIMEOUT waiting for $1"; return 1
}
sudo cp -a "$BK/etc-libreqos/." /etc/libreqos/
test -f "$BK/bridge_lkg.json" \
  && sudo cp -a "$BK/bridge_lkg.json" /var/lib/libreqos-bridge/bridge_lkg.json
for p in libreqos-preflight libreqos libreqos-dataplane; do
  ls "$BK"/${p}_*.deb >/dev/null 2>&1 || {
    echo "FAIL: backup is missing $p; cannot roll back without the old debs" >&2
    exit 1
  }
done
sudo dpkg -i "$BK"/libreqos-preflight_*.deb "$BK"/libreqos_*.deb \
  "$BK"/libreqos-dataplane_*.deb
sudo systemctl daemon-reload
sudo systemctl restart libreqos-lqosd && wait_active libreqos-lqosd 60
sudo systemctl restart libreqos-scheduler && wait_active libreqos-scheduler 60
sudo systemctl restart libreqos-bridge && wait_active libreqos-bridge 120
```

## v2.0 to v3 (staged)

Do not upgrade in place. Keep the v2 host recoverable and keep the **legacy v2
apt source and key available** until the rollback window closes; do not delete
them. `libreqos-v3-rollback --from-archive` resolves the v2 packages from the
legacy `repo.libreqos.com` archive and needs that source/key. If both sources
define `libreqos`, give the v3 source priority (a pin) rather than disabling
the v2 archive.

Run the v3 preflight, then follow the staged steps in
[Upgrading from v2.0](upgrade-from-v2.md): install the three packages at one
version, adopt the v2 config, bind the dataplane NICs, and verify. The v2
source stays available for rollback.

## Known beta16 limitation

On an **already-bound** NIC, `lqos_setup ports-plan --queues N` may report
`NIC queue count undetectable ... pass --queues N` even when `--queues` is
passed. This is the ordering bug tracked in **lqos-74t.89**; this page will be
updated when it lands.

The upgrade itself does **not** change the worker count: the existing
`lqos.conf` and generated plan are preserved. Do not change `[queues] workers`
on an already-bound beta16 host through the plan writer; do not hand-edit
`50-ports.conf`. If you need a worker change now, escalate rather than
following an untested pre-bind sequence.
