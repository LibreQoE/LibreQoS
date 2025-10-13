# LibreQoS Software Components

## Systemd Services
### lqosd

- Manages actual XDP code.
- Coded in Rust.
- Runs the GUI available at http://a.b.c.d:9123

### lqos_scheduler

- lqos_scheduler performs continuous refreshes of LibreQoS' shapers, including pulling from any enabled CRM Integrations (UISP, Splynx, Netzur).
- Actions:
  - On start: Run a full setup of queues
  - Every X minutes: Update queues, pulling new configuration from CRM integration, if enabled.
    - The default minute interval is 30, so the refresh occurs every 30 minutes by default.
    - The minute interval is adjustable with the setting `queue_refresh_interval_mins` in `/etc/lqos.conf`.

### Checking service status

```
sudo systemctl status lqosd lqos_scheduler
```

If the status of one of the two services shows 'failed', examine why using journalctl, which shows the full status of the service. For example, if lqosd failed, you would run:
```
sudo journalctl -u lqosd -b
```
Press the End key on the keyboard to take you to the bottom of the log to see the latest updates to that log.

Lqosd will provide specific reasons it failed, such as an interface not being up, an interface lacking multi-queue, or other cocnerns.

### Debugging lqos_scheduler

In the background, lqos_scheduler runs the Python script scheduler.py, which in turn runs the Python script LibreQoS.py

- scheduler.py: performs continuous refreshes of LibreQoS' shapers, including pulling from any enabled CRM Integrations (UISP, Splynx, Netzur).
- LibreQoS.py: creates and updates queues / shaping of devices

One-time runs of these individual components can be very helpful for debugging and to make sure everything is correctly configured.

First, stop lqos_scheduler

```shell
sudo systemctl stop lqos_scheduler
```

For one-time runs of LibreQoS.py, use

```shell
sudo ./LibreQoS.py
```

- To use the debug mode with more verbose output, use:

```shell
sudo ./LibreQoS.py --debug
```

To confirm that lqos_scheduler (scheduler.py) is able to work correctly, run:

```shell
sudo python3 scheduler.py
```

Once you have any errors eliminated, restart lqos_scheduler with

```shell
sudo systemctl start lqos_scheduler
```
