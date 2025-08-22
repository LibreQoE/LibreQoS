# Upgrading

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed the .deb

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget | deb_file_zip_url |
sudo apt-get install unzip
unzip | deb_file_zip_name |
sudo apt install ./| deb_file_name |
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
