# Upgrading

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed the .deb

```{important}
Starting in v2.0, mapped-circuit ingest depends on valid entitled license/grant state. Without a valid Insight or Local license/grant state, LibreQoS reads only the first 1000 valid mapped circuits into active shaping state. This includes expired or otherwise invalid local grant/license state. See [Insight Licensing Behavior](insight.md#mapped-circuit-limits-and-license-state) and [Troubleshooting](troubleshooting.md).

Starting in v2.2, the Node API uses the same mapped-circuit policy. Networks with 1,000 or fewer valid mapped circuits may use the API without an Insight subscription; larger networks require a valid API or Insight entitlement. API calls remain authenticated. Administrators can create up to 16 named local API keys in **License & Services**; keys are shown once, stored only as SHA-256 digests, and can be revoked. Existing license keys and the legacy local bearer token remain compatible, and key changes may take up to 30 seconds to take effect.
```

Run:

```bash
cd /tmp
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

Using `/tmp` avoids local `.deb` permission issues where `apt` cannot access a package stored in a private home directory as user `_apt`.

If `apt install` completes normally, restart services:

```bash
sudo systemctl restart lqosd lqos_scheduler
```

### Ubuntu 24.04 hotfix if the upgrade stops

On affected Ubuntu 24.04 hosts using `systemd-networkd`, `apt install` may stop with a hotfix requirement message. This is expected.

If that happens, run:

```bash
sudo /opt/libreqos/src/systemd_hotfix.sh install
sudo reboot
```

The hotfix installer bootstraps the signed LibreQoS APT repo at `https://repo.libreqos.com`, installs the patched Noble `systemd` package set, and pins those packages for future updates.
By default, it selects the current hotfix version from that repo.
If LibreQoS support tells you that your installed package is older than v2.2, use this recovery command instead:

```bash
sudo HOTFIX_PACKAGE_VERSION=255.4-1ubuntu9999+libreqos1 /opt/libreqos/src/systemd_hotfix.sh install
sudo dpkg --configure -a
sudo reboot
```

After the reboot, resume the upgrade and restart services:

```bash
cd /tmp
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
sudo systemctl restart lqosd lqos_scheduler
```

### Post-upgrade reboot

Now reboot the LibreQoS box with:

```bash
sudo reboot
```

This will flush the old eBPF maps and load the latest LibreQoS version.

Then continue with the normal post-upgrade validation below.

### Post-Upgrade Validation (Required)

Run these checks after upgrade/reboot:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "20 minutes ago"
```

Then verify:
1. WebUI Dashboard and Scheduler Status are healthy.
2. Integration users: a fresh sync produces expected `ShapedDevices.csv`/`network.json` behavior.
3. Topology depth matches your chosen integration strategy.

If checks fail, go directly to [Troubleshooting](troubleshooting.md) before further config changes.

## If you installed with Git

1. Change to your LibreQoS directory (e.g. `cd /opt/libreqos`)
2. Update from Git: `git pull`
3. ```git switch develop```
5. Recompile: `./build_rust.sh`
6. `sudo rust/remove_pinned_maps.sh`

### Ubuntu 24.04 hotfix for Git installs

Before restarting LibreQoS services on Ubuntu 24.04, check whether the Noble `systemd-networkd` hotfix should be offered:

```bash
cd /opt/libreqos/src
./systemd_hotfix.sh status
```

If the script reports that the hotfix should be offered, install it and reboot before continuing:

```bash
sudo ./systemd_hotfix.sh install
sudo reboot
```

The installer selects the current hotfix version from the signed LibreQoS APT repo. LibreQoS support may provide a `HOTFIX_PACKAGE_VERSION` override for a specific incident.

Run the following commands to reload the LibreQoS services.

```shell
sudo systemctl restart lqosd lqos_scheduler
```

### Notes on pinned BPF maps

`rust/remove_pinned_maps.sh` removes pinned maps used by the eBPF/XDP pipeline so newer map schemas can load cleanly after upgrades.

Recent versions include additional map cleanup (including `ip_mapping_epoch`). If this cleanup is skipped after a schema-changing upgrade, you may see stale behavior or mapping inconsistencies.

If you use Git installs, keep this order:
1. build/update binaries
2. stop/restart services as needed
3. run `sudo rust/remove_pinned_maps.sh`
4. restart `lqosd` and `lqos_scheduler`

## Stop-and-Triage Symptoms

Pause rollout and triage immediately if you see:

- `lqosd` or `lqos_scheduler` not healthy after restart
- unexpected hierarchy collapse/expansion after integration sync
- persistent scheduler unhealthy state in WebUI

Primary references:
- [Troubleshooting](troubleshooting.md)
- [CRM/NMS Integrations](integrations.md)
