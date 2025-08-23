# Upgrading

```{warning}
If you use the XDP bridge, traffic will briefly stop passing through the bridge when lqosd restarts (XDP bridge is only operating while lqosd runs).
```

## If you installed the .deb

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://libreqos.io/wp-content/uploads/2025/08/libreqos_1.5-RC1.202508232013-1_amd64.zip
sudo apt-get install unzip
unzip libreqos_1.5-RC1.202508232013-1_amd64.zip
sudo apt install ./libreqos_1.5-RC1.202508232013-1_amd64.deb
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
