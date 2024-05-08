# Updating 1.5 To Latest Version

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed with Git

1. Change to your `LibreQoS` directory (e.g. `cd /opt/LibreQoS`)
2. Update from Git: `git pull`
3. ```git switch develop```
4. Hack to fix Python 3.10 quirk (will fix later)
```
cd /opt/libreqos/src/rust
cargo update
sudo cp /usr/lib/x86_64-linux-gnu/libpython3.11.so /usr/lib/x86_64-linux-gnu/libpython3.10.so.1.0
```
5. Recompile: `./build-rust.sh`
6. `sudo rust/remove_pinned_maps.sh`

Run the following commands to reload the LibreQoS services.

```shell
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
sudo systemctl restart lqos_scheduler
```
