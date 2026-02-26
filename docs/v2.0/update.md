# Upgrading

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed the .deb

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
sudo systemctl restart lqosd lqos_scheduler
```

Now reboot the LibreQoS box with:
```
sudo reboot
```
This will flush the old eBPF maps and load the latest LibreQoS version.

## If you installed with Git

1. Change to your LibreQoS directory (e.g. `cd /opt/libreqos`)
2. Update from Git: `git pull`
3. ```git switch develop```
5. Recompile: `./build_rust.sh`
6. `sudo rust/remove_pinned_maps.sh`

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
