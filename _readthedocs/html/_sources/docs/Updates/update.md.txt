# Updating 1.4 To Latest Version

```{warning}
If you use the XDP bridge, traffic will stop passing through the bridge during the update (XDP bridge is only operating while lqosd runs).
```

## If you installed with Git

1. Change to your `LibreQoS` directory (e.g. `cd /opt/LibreQoS`)
2. Update from Git: `git pull`
3. Recompile: `./build-rust.sh`
4. `sudo rust/remove_pinned_maps.sh`

Run the following commands to reload the LibreQoS services.

```shell
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
sudo systemctl restart lqos_scheduler
```

## If you installed through the APT repository

All you should have to do in this case is run `sudo apt update && sudo apt upgrade` and LibreQoS should install the new package.
