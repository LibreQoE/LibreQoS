# Troubleshooting

## Start Here: Symptom Triage

Use this table to jump to the first checks quickly.

Need definitions for licensing/scheduler terms? See the [Glossary](glossary.md).

| Symptom | First check | WebUI location | Next section |
|---|---|---|---|
| Cannot access WebUI | `systemctl status lqosd caddy` | N/A (UI unavailable) | No WebUI at x.x.x.x:9123 or HTTPS URL |
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
ls -lh /opt/libreqos/src/topology_import.json /opt/libreqos/src/shaping_inputs.json
```

If you run a manual or custom-file deployment instead of a built-in integration, include:

```bash
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```

And include:
- current version/build
- integration type and strategy (if used)
- exact symptom and when it started

### User password not working

Current builds will:
- migrate older auth files automatically
- redirect `/login.html` to `/first-run.html` when no users exist

If the correct username/password still fails, first restart `lqosd` and try again:
```bash
sudo systemctl restart lqosd
```

Only remove `lqusers.toml` if you are intentionally resetting access or if the file is corrupt and cannot be repaired. After removing it, restart `lqosd` and open `BOX_IP:9123/login.html` if SSL is disabled; the WebUI should redirect you to first-run setup automatically.

### No WebUI at x.x.x.x:9123 or HTTPS URL

The WebUI is controlled by `lqosd`. If optional HTTPS with Caddy is enabled, `caddy` also has to be healthy.

Start by checking:
```
sudo systemctl status lqosd caddy
```

Then:

- If SSL is disabled, test `http://your-management-ip:9123/`
- If SSL is enabled with a hostname, test `https://your-hostname/`
- If SSL is enabled without a hostname, test `https://your-management-ip/`
- If browsers warn in local-certificate mode, trust `/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt` on the operator workstation

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

If the scheduler fails immediately after a restart with a message like `Socket (typically /run/lqos/bus) not found`, that indicates `lqosd` had not finished binding the local bus yet. Current builds wait briefly for bus readiness at scheduler startup instead of crashing immediately, so repeated startup panics after restart should no longer be expected.

If a host upgrades to a newer CPython 3.x minor release than the machine used to build LibreQoS, current packages should no longer require an exact Python minor match for `liblqos_python.so`. The extension is now built in PyO3 `abi3` mode with a Python 3.10 floor. If you still see interpreter crashes or import failures after such an upgrade, treat that as a bug and capture:

```bash
python3 --version
python3 - <<'PY'
import sysconfig
print(sysconfig.get_config_var("SOABI"))
PY
file /opt/libreqos/src/liblqos_python.so
ldd /opt/libreqos/src/liblqos_python.so
```

Routine package upgrades now keep `lqosd` in charge of the main WebUI when `/etc/lqos.conf` already exists. Current packages no longer start the dedicated `lqos_setup` web service during a normal upgrade just because newer first-run checks are incomplete. If the upgraded host still needs a first admin user or a topology source, finish that work in the normal WebUI (`first-run.html` or `Complete Setup`) instead of expecting `lqos_setup` to take over port `9123`.

If startup shaping fails because `shaping_inputs.json` is missing or stale, current builds leave the scheduler running in a degraded state and wait for the next scheduled full refresh to recover. The high-frequency topology refresh tick stays disabled until one shaping pass completes successfully, so repeated 3-second refresh attempts should not continue hammering a fresh install that has not produced runtime topology inputs yet.

If scheduler startup is degraded with a message about topology runtime still building, that usually means LibreQoS is still finishing an import or refreshing shaping data. Give the current cycle a little time to complete, then recheck Scheduler Status before changing unrelated settings.

If scheduler startup is degraded with a message that topology runtime failed for the current generation, inspect:

```bash
cat /opt/libreqos/src/topology_runtime_status.json
ls -lh /opt/libreqos/src/topology_effective_state.json /opt/libreqos/src/network.effective.json /opt/libreqos/src/shaping_inputs.json
journalctl -u lqos_scheduler --since "30 minutes ago"
```

If Topology Manager changes or imports seem stuck on older data, check whether LibreQoS set older snapshots aside under `/opt/libreqos/src/.topology_stale/`, then review recent scheduler logs before retrying.

If Insight topology looks wrong, review the current troubleshooting snapshot that `lqosd` is preparing for Insight:

```bash
cat /opt/libreqos/src/network.insight.debug.json
```

Treat `network.insight.debug.json` as a troubleshooting snapshot only; do not edit it.

If specific APs or switches appear multiple times with suffixed names such as `... [AP deadbeef]`, check whether UISP is returning duplicate rows for the same device ID. Current builds defensively deduplicate raw UISP devices by `identification.id` before topology graph construction, and skip any residual duplicate device IDs during graph assembly.

If an integration subprocess fails, current builds keep the scheduler alive, publish a shortened output preview to the scheduler status/error surfaces, and save the full captured output to a timestamped file under `/tmp` such as `lqos_scheduler_uisp_integration_YYYYMMDD_HHMMSS.log`.

### Scheduler status in WebUI looks unhealthy

Recent builds expose scheduler readiness/state in the WebUI (Node Manager).
If the scheduler is still starting, the sidebar now reports the current startup phase and a coarse progress ring rather than only a spinner.
Current builds also treat scheduler progress, output, and error bus messages as proof that the scheduler is alive, so the sidebar should not stay stuck on `Scheduler available: false` while the scheduler is actively reporting work.

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

### Site Map appears blank or slow

Site Map has one extra dependency beyond normal WebUI data feeds: current builds fetch bbox/bootstrap data and raster tiles from `https://insight.libreqos.com`.

If Site Map alone is blank or slow:
1. Confirm `lqosd` is healthy.
2. Confirm the box can reach `insight.libreqos.com` from its management network.
3. Confirm runtime topology still carries coordinates for mapped Sites/APs in `network.effective.json`.
4. Wait briefly and reload the page; the map page retries tile requests automatically while cold tiles are being populated upstream.
5. Check recent `lqosd` logs:
```
journalctl -u lqosd --since "10 minutes ago"
```

If the rest of WebUI is healthy but Site Map continues to fail, treat it as a map/tile dependency issue rather than a general scheduler or shaping failure.

### Virtual node promotion collision (network.json)

If LibreQoS.py fails with an error like `Virtual node promotion collision: 'AP_A' already exists at this level.`, you have a `"virtual": true` node whose children get promoted into a parent level where a node with the same name already exists.

Rename one of the colliding nodes (names must be unique among siblings after virtual-node promotion), or restructure the hierarchy so promoted children won’t collide.
For a visual of the logical-to-physical promotion flow and CPU placement, see [Advanced Configuration Reference](configuration-advanced.md).

### Mapped circuit limit reached

If logs mention messages like:
- `Mapped circuit limit reached`
- `Bakery mapped circuit cap enforced`

`ShapedDevices.csv` can contain unlimited entries, but without a valid Insight or Local license/grant state LibreQoS admits only the first 1000 valid mapped circuits into active shaping state.

The default 1000 mapped-circuit limit applies when license/grant state is:
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
1. Confirm license status in the `License & Services` page.
2. Review `lqosd` logs for requested/allowed/dropped counts.
3. Reduce mapped circuit count (short term) or update licensing/limits (long term).

### Urgent issue codes and first actions

WebUI urgent issues include machine-readable codes. Use them to triage quickly.

| Code | Meaning | First checks | Typical fix path |
|---|---|---|---|
| `MAPPED_CIRCUIT_LIMIT` | Bakery is enforcing a mapped-circuit limit. | Insight license status, `journalctl -u lqosd` for requested/allowed/dropped counts. | Reduce mapped circuits immediately or update license/limits. |
| `TC_U16_OVERFLOW` | Queue/class minor IDs exceeded the Linux tc u16 range on a CPU queue. | `journalctl -u lqos_scheduler -u lqosd`, topology depth/queue distribution. | Increase queue count and/or simplify/rebalance hierarchy (for example with integration strategy or root promotion changes). |
| `TC_QDISC_CAPACITY` | Planned auto-allocated qdiscs exceed the per-interface safe budget or Bakery's conservative memory-safety preflight before apply. | Estimated per-interface qdisc counts, qdisc-kind breakdown, and memory fields in the urgent issue context, `journalctl -u lqos_scheduler -u lqosd`, `on_a_stick` and `queue_mode` config. | Reduce the planned qdisc load for this run (for example fewer circuits/devices in the test shape) before retrying; do not trust partial apply. |
| `BAKERY_MEMORY_GUARD` | A chunked Bakery full reload was stopped mid-apply because available host memory fell below the safety floor. | `journalctl -u lqosd`, available/total memory in the urgent issue context, and recent Bakery apply progress. | Treat the run as failed, reduce memory pressure or queue footprint, and retry only after the host is stable. |
| `XDP_IP_MAPPING_CAPACITY` | Required IP mappings exceed the current XDP kernel map capacity. | `ShapedDevices.csv` row shape, IPv4/IPv6 mix, one-device-vs-many-device assumptions, `journalctl -u lqos_scheduler -u lqosd`. | Reduce required mappings immediately (for example fewer devices or IPv4-only test shape), or raise kernel map capacity in a coordinated change. |
| `XDP_IP_MAPPING_APPLY_FAILED` | One or more IP mapping inserts failed during apply. | `journalctl -u lqos_scheduler -u lqosd` for summarized failure examples and counts. | Fix the underlying mapping failure, then rerun; do not trust partial shaping. |

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
