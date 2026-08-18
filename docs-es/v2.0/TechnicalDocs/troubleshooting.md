# Troubleshooting

## Common Issues

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

At the command-line, run ```sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd``` which runs lqosd manually, and will provide specifics regarding why it failed to start.

### RTNETLINK answers: Invalid argument

This tends to show up when the MQ qdisc cannot be added correctly to the NIC interface. This would suggest the NIC has insufficient RX/TX queues. Please make sure you are using the [recommended NICs](../../SystemRequirements/Compute.md#network-interface-requirements).

### Python dependency or virtual environment errors

Packaged installs keep LibreQoS Python dependencies in `/opt/libreqos/venv`. The services still run as root, but Python packages do not mix with apt-managed system packages. If the scheduler reports missing Python modules, or package configuration was interrupted while installing Python dependencies, rebuild the virtual environment:

```bash
sudo /opt/libreqos/src/bin/rebuild_python_venv.sh
sudo dpkg --configure -a
sudo systemctl restart lqosd lqos_scheduler
```

Git-based installs should use `./build_rust.sh` after pulling updates. It rebuilds the virtual environment before refreshing service files or restarting services. If systemd reports `status=203/EXEC` on `/opt/libreqos/venv/bin/python`, or a failed scheduler pre-start check, rebuild the virtual environment with the command above and restart `lqos_scheduler`.

Older installs that predate the virtual environment may show `ModuleNotFoundError` and suggest system `pip` commands. Do not repair current installs with system `pip` or `--break-system-packages`; those packages are not used by the venv-backed scheduler service. Upgrade to a package that creates `/opt/libreqos/venv`, then use the repair command above.

### All customer IPs are listed under Unknown IPs, rather than Shaped Devices in GUI

```
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo /opt/libreqos/venv/bin/python /opt/libreqos/src/LibreQoS.py
```

The console output from running LibreQoS.py directly provides more specific errors regarding issues with ShapedDevices.csv and network.json
Once you have identified the error and fixed ShapedDevices.csv and/or Network.json, please then run

```sudo systemctl start lqos_scheduler```
