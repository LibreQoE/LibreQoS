# Troubleshooting

## Start Here: Symptom Triage

Use this table to jump to the first checks quickly.

Need definitions for licensing/scheduler terms? See the [Glossary](glossary.md).

| Symptom | First check | WebUI location | Next section |
|---|---|---|---|
| Cannot access WebUI | `systemctl status lqosd` | N/A (UI unavailable) | No WebUI at x.x.x.x:9123 |
| Traffic is not shaping | verify `to_internet` / `to_network`, service state | WebUI Dashboard | LibreQoS Is Running, But Traffic Not Shaping |
| Scheduler appears unhealthy | check `lqosd` and `lqos_scheduler` logs | WebUI -> Scheduler Status | Scheduler status in WebUI looks unhealthy |
| Topology/flow views blank | confirm recent traffic and `lqosd` health | WebUI -> Flow Globe / Tree / ASN Analysis | Flow Globe / Tree Overview / ASN Analysis appears blank |
| Urgent issue code appears | open issue details and map code | WebUI -> Urgent Issues | Urgent issue codes and first actions |
| Mapped circuit cap events | validate license state and mapped counts | Insight UI + WebUI -> Urgent Issues | Mapped circuit limit reached |

## Common Issues

### Where in WebUI

- Service/health overview: `WebUI -> Dashboard`
- Scheduler readiness: `WebUI -> Scheduler Status`
- High-priority alerts: `WebUI -> Urgent Issues`
- Topology/traffic visualization: `WebUI -> Network Tree Overview` and `Flow Globe`
- Shaped records review: `WebUI -> Shaped Devices Editor`

### Before asking in chat: collect this evidence

Collect these first to reduce back-and-forth:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd --since "30 minutes ago"
journalctl -u lqos_scheduler --since "30 minutes ago"
```

If integration-related, also include:

```bash
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```

And include:
- current version/build
- integration type and strategy (if used)
- exact symptom and when it started

### User password not working

Delete the lqusers file:
```
sudo rm /opt/libreqos/src/lqusers.toml
sudo systemctl restart lqosd lqos_scheduler
```
Then visit: BOX_IP:9123/index.html
This will allow you to set up the user again from scratch using the WebUI.

### No WebUI at x.x.x.x:9123

The WebUI is controlled by the `lqosd` service. In current builds, most WebUI access failures are caused by `lqosd` not being healthy.

Start by checking:
```
sudo systemctl status lqosd
```

Then follow the full workflow in **Service lqosd is not running or failed to start** below.

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

### On-a-stick shaping looks wrong or one direction is weak

On-a-stick mode depends on queue splitting per direction. If TX queue discovery is wrong or `override_available_queues` is mis-set, directional mapping can be degraded.

Check:
```
sudo systemctl status lqosd
journalctl -u lqosd --since "10 minutes ago"
```

Then verify queue-related config in `/etc/lqos.conf` and restart:
```
sudo systemctl restart lqosd lqos_scheduler
```

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

Lqosd will provide specific reasons it failed, such as an interface not being up, an interface lacking multi-queue, or other concerns.

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

### Scheduler status in WebUI looks unhealthy

Recent builds expose scheduler readiness/state in the WebUI (Node Manager).

If scheduler status appears down/stale:
1. Verify both services:
```
sudo systemctl status lqosd lqos_scheduler
```
2. Check recent scheduler logs:
```
journalctl -u lqos_scheduler --since "30 minutes ago"
```
3. Check lqosd bus/log state for scheduler-ready or scheduler-error messages:
```
journalctl -u lqosd --since "30 minutes ago"
```
4. If config/integration changes were recent, restart services cleanly:
```
sudo systemctl restart lqosd lqos_scheduler
```

If status repeatedly oscillates between ready/error, collect both logs and confirm integration credentials/timeouts in `/etc/lqos.conf`.

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

### Flow Globe / Tree Overview / ASN Analysis appears blank

Some views require enough recent data to render meaningfully. If pages look empty:
1. Confirm `lqosd` is healthy.
2. Wait for traffic/data to accumulate.
3. Reload the page after 1-2 minutes.
4. Check logs for websocket or ticker warnings:
```
journalctl -u lqosd --since "10 minutes ago"
```

If still blank under normal traffic, collect recent logs and open an issue.

### Virtual node promotion collision (network.json)

If LibreQoS.py fails with an error like `Virtual node promotion collision: 'AP_A' already exists at this level.`, you have a `"virtual": true` node whose children get promoted into a parent level where a node with the same name already exists.

Rename one of the colliding nodes (names must be unique among siblings after virtual-node promotion), or restructure the hierarchy so promoted children won’t collide.
For a visual of the logical-to-physical promotion flow and CPU placement, see [Advanced Configuration Reference](configuration-advanced.md).

### Mapped circuit limit reached

If logs mention messages like:
- `Mapped circuit limit reached`
- `Bakery mapped circuit cap enforced`

`ShapedDevices.csv` can contain unlimited entries, but without a valid Insight subscription/license LibreQoS admits only the first 1000 valid mapped circuits into active shaping state.

The default 1000 mapped-circuit limit applies when Insight is:
- missing
- expired
- otherwise invalid
- operating with offline-invalid local grant state

Typical operator-visible symptoms:
- prominent mapped-circuit-limit warning in WebUI
- left-hand navigation usage indicator showing approach to or exhaustion of the 1000 limit
- `journalctl -u lqosd` messages showing requested/allowed/dropped mapped counts
- partial shaping, with circuits beyond the active limit left outside shaping state

Recommended checks:
1. Confirm Insight/license status in the UI.
2. Review `lqosd` logs for requested/allowed/dropped counts.
3. Reduce mapped circuit count (short term) or update licensing/limits (long term).

### Urgent issue codes and first actions

WebUI urgent issues include machine-readable codes. Use them to triage quickly.

| Code | Meaning | First checks | Typical fix path |
|---|---|---|---|
| `MAPPED_CIRCUIT_LIMIT` | Bakery is enforcing a mapped-circuit limit. | Insight license status, `journalctl -u lqosd` for requested/allowed/dropped counts. | Reduce mapped circuits immediately or update license/limits. |
| `TC_U16_OVERFLOW` | Queue/class minor IDs exceeded the Linux tc u16 range on a CPU queue. | `journalctl -u lqos_scheduler -u lqosd`, topology depth/queue distribution. | Increase queue count and/or simplify/rebalance hierarchy (for example with integration strategy or root promotion changes). |

Operational pattern:
1. Open urgent issue details in WebUI (code/message/context).
2. Pull matching logs from `lqosd` and `lqos_scheduler`.
3. Apply the immediate mitigation.
4. Acknowledge/clear the issue in UI once stable.

## Related Pages

- [Quickstart](quickstart.md)
- [Configure LibreQoS](configuration.md)
- [CRM/NMS Integrations](integrations.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Performance Tuning](performance-tuning.md)
