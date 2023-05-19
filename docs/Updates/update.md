## Updating 1.4 To Latest Version

Note: If you use the XDP bridge, traffic will stop passing through the bridge during the update (XDP bridge is only operating while lqosd runs).

1. Change to your `LibreQoS` directory (e.g. `cd /opt/LibreQoS`)
2. Update from Git: `git pull`
3. Recompile: `./build-rust.sh`
4. `sudo rust/remove_pinned_maps.sh`
5.

```
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
sudo systemctl restart lqos_scheduler
```
