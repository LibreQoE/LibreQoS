# Troubleshooting

## Common Issues

### User password not working

Delete the lqusers file:
```
sudo rm /opt/libreqos/src/lqusers.toml
sudo systemctl restart lqosd lqos_scheduler
```
Then visit: BOX_IP:9123/index.html
This will allow you to set up the user again from scratch using the WebUI.

### No WebUI at x.x.x.x:9123

The WebUI is controlled by the lqosd service. Usually, when the WebUI doesn't start, it is related to lqosd being in a failed state.
Check to see if the lqosd service is running:
```
sudo systemctl status lqosd
```

If the status is 'failed', examine why using journalctl, which shows the full status of the service:
```
journalctl -u lqosd --since "10 minutes ago"
```
Press the End key on the keyboard to take you to the bottom of the log to see the latest updates to that log.

Lqosd will provide specific reasons it failed, such as an interface not being up, an interface lacking multi-queue, or other cocnerns.

### LibreQoS Is Running, But Traffic Not Shaping

In /etc/lqos.conf, ensure that `to_internet` and `to_network` are set correctly. If not, simply swap the interfaces between those and restart lqosd and the scheduler.

```
sudo systemctl restart lqosd lqos_scheduler
```

Make sure your services are running properly

```
sudo systemctl status lqosd lqos_scheduler
```

The service lqos_scheduler is dependent on the lqosd service being in a healthy, running state.

### Service lqosd is not running or failed to start

Check to see the state of the lqosd service:
```
sudo systemctl status lqosd
```

If the status is 'failed', examine why using journalctl, which shows the full status of the service:
```
journalctl -u lqosd --since "10 minutes ago"
```
Press the End key on the keyboard to take you to the bottom of the log to see the latest updates to that log.

Lqosd will provide specific reasons it failed, such as an interface not being up, an interface lacking multi-queue, or other cocnerns.

### Advanced lqosd debug

At the command-line, run:
```
sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd
```
which runs lqosd in debug mode, and will provide specifics regarding why it failed to start.

### Service lqos_scheduler shows errors

If `sudo systemctl status lqosd lqos_scheduler` reveals that the lqos_scheduler service has issues, you can export a comprehensive log of the errors to a file:
```
sudo journalctl -u lqos_scheduler --since "1 day ago" --no-pager > lqos_sched_log.txt
```
This exports a log file to lqos_sched_log.txt. You can review this file to see what caused the scheduler to error out.

### RTNETLINK answers: Invalid argument

This tends to show up when the MQ qdisc cannot be added correctly to the NIC interface. This would suggest the NIC has insufficient RX/TX queues. Please make sure you are using the [recommended NICs](requirements.md).

### Python ModuleNotFoundError in Ubuntu 24.04
```
pip uninstall binpacking --break-system-packages --yes
sudo pip uninstall binpacking --break-system-packages --yes
sudo pip install binpacking --break-system-packages
pip uninstall apscheduler --break-system-packages --yes
sudo pip uninstall apscheduler --break-system-packages --yes
sudo pip install apscheduler --break-system-packages
pip uninstall deepdiff --break-system-packages --yes
sudo pip uninstall deepdiff --break-system-packages --yes
sudo pip install deepdiff --break-system-packages
```
### All customer IPs are listed under Unknown IPs, rather than Shaped Devices in GUI
```
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo python3 LibreQoS.py
```

The console output from running LibreQoS.py directly provides more specific errors regarding issues with ShapedDevices.csv and network.json
Once you have identified the error and fixed ShapedDevices.csv and/or Network.json, please then run

```sudo systemctl start lqos_scheduler```

### Virtual node promotion collision (network.json)

If LibreQoS.py fails with an error like `Virtual node promotion collision: 'AP_A' already exists at this level.`, you have a `"virtual": true` node whose children get promoted into a parent level where a node with the same name already exists.

Rename one of the colliding nodes (names must be unique among siblings after virtual-node promotion), or restructure the hierarchy so promoted children won’t collide.

### Systemd segfault

If you experience a segfault in systemd, this is a known issue in systemd [1](https://github.com/systemd/systemd/issues/36031) [2](https://github.com/systemd/systemd/issues/33643).
To work around it, you can compile systemd from scratch:

### Install build dependencies

```
sudo apt update
sudo apt install build-essential git meson libcap-dev libmount-dev libseccomp-dev \
libblkid-dev libacl1-dev libattr1-dev libcryptsetup-dev libaudit-dev \
libpam0g-dev libselinux1-dev libzstd-dev libcurl4-openssl-dev
```

#### Clone systemd repository from github

```
git clone https://github.com/systemd/systemd.git
cd systemd
git checkout v257.5
meson setup build
meson compile -C build
sudo meson install -C build
```

Then, reboot, and confirm the systemd version with `systemctl --version`

```
libreqos@libreqos:~$ systemctl --version
systemd 257 (257.5)
```
