# Installing LibreQoS v3

LibreQoS v3 is the DPDK dataplane plus the `lqosd` control plane. Pick the path
that matches your host:

- **Fresh install**: a new Ubuntu 24.04 host (below).
- **Beta-to-beta upgrade**: a host already running a v3 beta (below).
- **v2.0 to v3**: a production v2 host. See
  [Upgrading from v2.0](upgrade-from-v2.md).

## Before you start

- Ubuntu Server 24.04 LTS or newer, with root/sudo.
- Two supported dataplane NICs, or one for an on-a-stick trunk. The dataplane
  NICs must have no management address and no bridge/bond/VLAN on top of them.
- Hugepages: the installer reserves 3072 × 2M pages. If that fails, add
  `default_hugepagesz=2M hugepages=3072` to the kernel command line and reboot.
- Proxmox/VM: use the modern virtio device `1af4:1041` with
  `disable-legacy=on` on both dataplane NICs.
- If this host once ran v2, an old `/etc/lqos.conf` may still exist. v3 ignores
  it and reads `/etc/libreqos/lqos.conf`.
- The install needs no internet access beyond the LibreQoS apt repository:
  Python dependencies ship inside the `libreqos` package.

## Fresh install

```bash
# 1. Add the LibreQoS v3 repository
curl -fsSL https://packages.libreqos.com/keys/libreqos-packages-keyring.gpg \
  | sudo tee /usr/share/keyrings/libreqos-packages-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/libreqos-packages-keyring.gpg] https://packages.libreqos.com beta main" \
  | sudo tee /etc/apt/sources.list.d/libreqos.list
sudo apt-get update

# 2. Check the host (PASS / WARN / BLOCK; stop on BLOCK)
sudo apt-get install -y libreqos-preflight && libreqos-v3-preflight

# 3. Install
sudo apt-get install -y libreqos libreqos-dataplane

# 4. First-run setup: NICs, rates, users, activate
sudo lqos_setup tui
```

`libreqos-dataplane` requires the exactly matching `libreqos` version, so apt
always installs a matched set.

The setup wizard picks the NIC pair, writes the rates to
`/etc/libreqos/lqos.conf`, and starts the services. Then
{ref}`verify the install <v3-verify>`.

### Setting the worker count

By default the wizard sizes the DPDK worker count from the link rate, CPU cores
and NIC queues. To use a fixed count instead, add it to `/etc/libreqos/lqos.conf`
before you confirm the wizard's final activation step:

```toml
[queues]
workers = 8
```

Changing the count on a host that is already running is not supported yet; see
[Known limitations](known-limitations.md).

(v3-verify)=
## Verify

```bash
dpkg -l 'libreqos*' | grep ^ii
systemctl is-active libreqos-lqosd libreqos-scheduler libreqos-bridge
journalctl -u libreqos-bridge --since "-10 min" | grep FATAL
```

Expect the same version on all three packages, `active` three times, and no
`FATAL` lines. The bridge may restart once or twice during its first activation;
that is normal as long as it is `active` now. If it is `failed`, run
`sudo systemctl restart libreqos-bridge` once and check again. If it still
fails, see [Troubleshooting](troubleshooting.md).

(v3-beta-upgrade)=
## Beta-to-beta upgrade

Paste the blocks, in order, into one root shell (`sudo -i`); later blocks use
the variables earlier ones set. A block that prints `FAIL` exits that shell
before it changes anything else: fix the cause, open a new root shell and start
again from step 1.

**1. Check the target.** All three packages must be available at the same
version:

```bash
V=$(apt-cache policy libreqos | awk '/Candidate:/ {print $2}')
case "$V" in ""|"(none)") echo "FAIL: no libreqos candidate"; exit 1;; esac
for p in libreqos-preflight libreqos libreqos-dataplane; do
  apt-cache madison "$p" | awk -F'|' '{gsub(/ /,"",$2); print $2}' \
    | grep -qxF "$V" || { echo "FAIL: $p=$V is not in the repository"; exit 1; }
done
echo "target: $V (all three packages present)"
```

**2. Back up** the config, the last-known-good (LKG) file and the three
current packages. The block checks every copy before it reports success:

```bash
OLD=$(dpkg-query -W -f='${Version}' libreqos 2>/dev/null || true)
test -n "$OLD" || { echo "FAIL: libreqos is not installed; this is not an upgrade"; exit 1; }
LKG=/var/lib/libreqos-bridge/bridge_lkg.json
test -f "$LKG" || { echo "FAIL: no $LKG on this host; do not upgrade without it"; exit 1; }
BK=/var/backups/libreqos-upgrade-$(date +%Y%m%dT%H%M%S)
install -d -m 0700 "$BK"
cp -a /etc/libreqos "$BK/etc-libreqos" && cp -a "$LKG" "$BK/bridge_lkg.json" \
  && diff -r /etc/libreqos "$BK/etc-libreqos" >/dev/null && cmp -s "$LKG" "$BK/bridge_lkg.json" \
  || { echo "FAIL: config/LKG backup in $BK is incomplete"; exit 1; }
# The beta repository usually keeps only the newest version, so the download
# can fail. Then set DEBSRC to a directory holding your own copies of the three
# $OLD debs (DEBSRC=/path/to/debs) and start again from step 1.
if [ -n "${DEBSRC:-}" ]; then cp -a "$DEBSRC"/*.deb "$BK/"
else ( cd "$BK" && apt-get download libreqos-preflight="$OLD" libreqos="$OLD" \
         libreqos-dataplane="$OLD" ) \
  || { echo "FAIL: cannot download the $OLD debs; set DEBSRC (see above)"; exit 1; }
fi
for p in libreqos-preflight libreqos libreqos-dataplane; do
  n=0
  for f in "$BK"/*.deb; do
    [ -f "$f" ] || continue
    [ "$(dpkg-deb -f "$f" Package)" = "$p" ] && [ "$(dpkg-deb -f "$f" Version)" = "$OLD" ] && n=$((n+1))
  done
  [ "$n" -eq 1 ] || { echo "FAIL: $BK needs exactly one $p=$OLD deb (found $n)"; exit 1; }
done
echo "$OLD" > "$BK/old-version"
echo "backup OK: $BK (old version $OLD)"
```

Do not upgrade until this prints `backup OK`.

**3. Upgrade** to exactly `$V`. The packages restart the services themselves:

```bash
apt-get update
NEEDRESTART_MODE=l apt-get install -y -o Dpkg::Options::=--force-confold \
  libreqos-preflight="$V" libreqos="$V" libreqos-dataplane="$V"
```

`--force-confold` keeps your edited config files. `NEEDRESTART_MODE=l` stops
`needrestart` from also restarting unrelated system services such as
`systemd-resolved`.

**4. Verify** with the {ref}`same checks <v3-verify>`. On the first boot after
an upgrade the bridge may log `section_validation=rejected:placement_budget`
followed by `Placement ownership ACTIVE`. That is the bridge re-placing circuits
across workers, not a failure.

**Rollback.** List the backups and pick one. The block works in a fresh shell,
and it re-checks the backup before it touches anything:

```bash
ls -dt /var/backups/libreqos-upgrade-*     # newest first
BK=/var/backups/libreqos-upgrade-YYYYMMDDTHHMMSS   # replace with the one you picked
OLD=$(cat "$BK/old-version" 2>/dev/null) || { echo "FAIL: $BK is not a verified backup"; exit 1; }
test -d "$BK/etc-libreqos" && test -f "$BK/bridge_lkg.json" \
  || { echo "FAIL: $BK has no config/LKG copy"; exit 1; }
DEBS=()
for p in libreqos-preflight libreqos libreqos-dataplane; do
  m=()
  for f in "$BK"/*.deb; do
    [ -f "$f" ] || continue
    [ "$(dpkg-deb -f "$f" Package)" = "$p" ] && [ "$(dpkg-deb -f "$f" Version)" = "$OLD" ] && m+=("$f")
  done
  [ "${#m[@]}" -eq 1 ] || { echo "FAIL: $BK needs exactly one $p=$OLD deb (found ${#m[@]})"; exit 1; }
  DEBS+=("${m[0]}")
done
read -r -p "Roll back to $OLD from $BK? [y/N] " a; [ "$a" = y ] || exit 1
cp -a "$BK/etc-libreqos/." /etc/libreqos/
cp -a "$BK/bridge_lkg.json" /var/lib/libreqos-bridge/bridge_lkg.json
dpkg -i "${DEBS[@]}"
systemctl daemon-reload
systemctl restart libreqos-lqosd libreqos-scheduler libreqos-bridge
```
