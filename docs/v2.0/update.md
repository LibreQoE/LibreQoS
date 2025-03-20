# Updating To The Latest Version

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed the .deb

Donwload the latest .deb [here](quickstart.md).

Unzip the .zip file and transfer the .deb to your LibreQoS box, installing with:
```
sudo apt install ./[deb file name]
```

Now reboot the LibreQoS box with:
```
sudo reboot
```
This will flush the old eBPF maps and load the latest LibreQoS version.

## If you installed with Git

1. Change to your `LibreQoS` directory (e.g. `cd /opt/LibreQoS`)
2. Update from Git: `git pull`
3. ```git switch develop```
5. Recompile: `./build-rust.sh`
6. `sudo rust/remove_pinned_maps.sh`

Run the following commands to reload the LibreQoS services.

```shell
sudo systemctl restart lqosd lqos_scheduler
```
