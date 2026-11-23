# Installing LibreQoS v3

LibreQoS v3 is the DPDK dataplane plus the `lqosd` control plane. Pick the path
that matches your host:

- **Fresh install**: a new Ubuntu Server 26.04 host (below).
- **Upgrade**: a host already running v3 (below).
- **v2.0 to v3**: a production v2 host. See
  [Upgrading from v2.0](upgrade-from-v2.md).

## Before you start

See [System Requirements](requirements.md) for the hardware. In short:

- Ubuntu Server 26.04 LTS (recommended) with root/sudo. 24.04 LTS is still
  supported.
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

## Release channels

The LibreQoS v3 repository has three channels (apt suites). Use `stable`
unless LibreQoS support asks you to test a release candidate.

| Channel | What it carries | Who should use it |
|---|---|---|
| `stable` | Released versions | Production hosts |
| `beta` | Release candidates, before they reach `stable` | Test and canary hosts |
| `nightly` | The current development tree, built daily | LibreQoS developers only; expect breakage |

The channel is the word after the URL in
`/etc/apt/sources.list.d/libreqos.list`. A host stays on its channel until you
change that line.

**Moving a v3 beta host to `stable`:**

```bash
sudo sed -i 's# beta main# stable main#' /etc/apt/sources.list.d/libreqos.list
sudo apt-get update && sudo apt-get upgrade
```

Every `stable` build is newer than the betas before it, so this is an ordinary
upgrade.

## Fresh install

```bash
# 1. Add the LibreQoS v3 repository
curl -fsSL https://packages.libreqos.com/keys/libreqos-packages-keyring.gpg \
  | sudo tee /usr/share/keyrings/libreqos-packages-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/libreqos-packages-keyring.gpg] https://packages.libreqos.com stable main" \
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
## Upgrade

```bash
sudo apt-get update && sudo apt-get upgrade
```

The packages restart the services themselves. Keep your edited config files
if apt asks about them. Then run the {ref}`same checks <v3-verify>`.

If apt reports `libreqos` packages as "kept back", install them by name
(`libreqos-dataplane` must always match the `libreqos` version):

```bash
sudo apt-get install libreqos libreqos-dataplane libreqos-preflight
```

On the first start after an upgrade the bridge may log
`section_validation=rejected:placement_budget` followed by
`Placement ownership ACTIVE`. That is the bridge re-placing circuits across
workers, not a failure.
