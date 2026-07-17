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

If the log shows `LibreQoS failed to attach the XDP/TC kernel` or `Unable to load the XDP/TC kernel`, treat `lqosd` startup as failed. The WebUI and local bus will not start until the kernel program loads and attaches successfully. The load error includes the raw return value, errno number, and errno code, for example `raw=-11, errno=11, code=EAGAIN`. Check for an existing XDP program, busy TC hook, missing driver support, or stale pinned maps before restarting `lqosd`.

If `journalctl -u lqosd` shows `lqosd host memory pressure` or `lqosd process memory critical`, the daemon detected high memory usage and logged diagnostic context. The watchdog does not restart `lqosd`; it records available memory, total memory, `lqosd` RSS/swap, thread count, flow count, and timing counters that help diagnose the source of memory growth. Host memory pressure is logged when available memory is below 10% of installed RAM. Process memory is logged as critical when `lqosd` RSS plus swap reaches 90% of installed RAM.

You can disable these diagnostics with a systemd environment override during short troubleshooting windows:

```bash
sudo systemctl edit lqosd
```

Set `LQOSD_MEMORY_WATCHDOG_DISABLED=1` only when you are actively watching memory pressure through another tool.

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

If startup shaping begins before topology runtime has finished publishing the current generation of `shaping_inputs.json`, current builds keep the scheduler in a startup wait state and retry the initial shaping pass every few seconds. A short `still building outputs for the current source generation` message right after restart now usually means LibreQoS is still finishing the import/runtime publish cycle, not that shaping is stuck until the next 30-minute refresh.

If a scheduled integration refresh lands while topology runtime is still publishing outputs for the new source generation, current builds keep the scheduler in a waiting state for that generation and retry the scheduled shaping refresh automatically as soon as topology runtime finishes. Treat `Scheduled shaping refresh deferred` as a transient wait only when the message says topology runtime is still building outputs for the current generation. If the message instead says topology runtime failed for the current generation, investigate that runtime failure directly.

If scheduler startup stays in that wait state for an unusually long time, or degrades with a message that topology runtime failed for the current generation, inspect:

```bash
cat /opt/libreqos/state/topology/topology_runtime_status.json
ls -lh /opt/libreqos/state/topology/topology_effective_state.json /opt/libreqos/state/topology/network.effective.json /opt/libreqos/state/shaping/shaping_inputs.json
journalctl -u lqos_scheduler --since "30 minutes ago"
journalctl -u lqosd --since "30 minutes ago"
```

If Topology Manager changes or imports seem stuck on older data, check whether LibreQoS set older snapshots aside under `/opt/libreqos/src/.topology_stale/`, then review recent scheduler and `lqosd` logs before retrying.

If Insight topology looks wrong, review the current troubleshooting snapshot that `lqosd` is preparing for Insight:

```bash
cat /opt/libreqos/src/network.insight.debug.json
```

Treat `network.insight.debug.json` as a troubleshooting snapshot only; do not edit it.

If `journalctl -u lqosd` shows repeated `BeginIngest queue full`, `IngestChunk queue full`, or `EndIngest queue full` warnings during startup or immediately after a topology import, older builds were dropping Insight ingest frames because the node's outbound control-channel queue was too small for burst uploads. Current builds apply backpressure on the Insight socket for ingest batches instead, so those warnings should no longer be expected during short import/startup bursts. If they still appear after upgrading, inspect recent `lqosd` CPU pressure and control-channel connectivity before assuming shaping itself is unhealthy.

If specific APs or switches appear multiple times with suffixed names such as `... [AP deadbeef]`, check whether UISP is returning duplicate rows for the same device ID. Current builds defensively deduplicate raw UISP devices by `identification.id` before topology graph construction, and skip any residual duplicate device IDs during graph assembly.

If an integration subprocess fails, current builds keep the scheduler alive, publish a shortened output preview to the scheduler status/error surfaces, and save the full captured output to a timestamped file under `/tmp` such as `lqos_scheduler_uisp_integration_YYYYMMDD_HHMMSS.log`. If shaping can continue from the last-known-good topology, the scheduler may still report ready, but the latest integration failure remains visible in scheduler status until the next successful integration run.

If the scheduler cannot read `lqos_overrides.json` or its materialized override layers because another process holds the overrides lock, current builds retry briefly and then block that reload. The previous topology remains in use, and the scheduler error includes lock-holder details such as PID, process name, operation, and lock creation time when available.

### Scheduler status in WebUI looks unhealthy

Recent builds expose scheduler readiness/state in the WebUI (Node Manager).
If the scheduler is still starting, the sidebar now reports the current startup phase and a coarse progress ring rather than only a spinner.
Current builds also treat scheduler progress, output, and error bus messages as proof that the scheduler is alive, so the sidebar should not stay stuck on `Scheduler available: false` while the scheduler is actively reporting work.
If the scheduler modal says scheduler details timed out, current builds keep showing the last good scheduler snapshot with its age instead of turning that transport problem into a scheduler error. Treat that warning as a WebUI or `lqosd` communication issue first, then confirm scheduler health in the service logs before assuming shaping failed.

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

### Python dependency or virtual environment errors

Packaged installs keep LibreQoS Python dependencies in `/opt/libreqos/venv`. The services still run as root, but Python packages do not mix with apt-managed system packages. If the scheduler reports missing Python modules, or package configuration was interrupted while installing Python dependencies, rebuild the virtual environment:

```bash
sudo /opt/libreqos/src/bin/rebuild_python_venv.sh
sudo dpkg --configure -a
sudo systemctl restart lqosd lqos_scheduler
```

Git-based installs should use `./build_rust.sh` after pulling updates. It rebuilds the virtual environment before refreshing service files or restarting services. If systemd reports `status=203/EXEC` on `/opt/libreqos/venv/bin/python`, or a failed scheduler pre-start check, rebuild the virtual environment with the command above and restart `lqos_scheduler`.

For manual shaping tests, use the same interpreter as the service:

```bash
sudo systemctl stop lqos_scheduler
sudo /opt/libreqos/venv/bin/python /opt/libreqos/src/LibreQoS.py
sudo systemctl start lqos_scheduler
```

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

### Circuits show Generated_PN instead of network.json node names

If a DIY/manual deployment uses `network.json` and `ShapedDevices.csv`, but the Circuit page shows parents such as `Generated_PN_1`, check the runtime shaping inputs:

```bash
jq '.circuits[] | {circuit_id, logical_parent_node_name, effective_parent_node_name, resolution_source}' /opt/libreqos/state/shaping/shaping_inputs.json
```

If `resolution_source` is `flat_bucket`, LibreQoS is in flat topology mode. That mode intentionally assigns circuits to generated CPU bucket queues instead of shaping them under the named `Parent Node` hierarchy.

To shape circuits under the node names from `network.json`, set:

```toml
[topology]
compile_mode = "full"
```

Then reload or wait for the scheduler to regenerate `network.effective.json` and `shaping_inputs.json`.

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

The Node API follows the same count starting with LibreQoS 2.2. At 1,000 or fewer mapped circuits, confirm that a named local API key (or compatible legacy/license key) is configured and that `lqos_api` is running. New and revoked named keys may take up to 30 seconds to take effect. Above 1,000, also confirm that the effective API or Insight entitlement is valid.

### Urgent issue codes and first actions

WebUI urgent issues include machine-readable codes. Use them to triage quickly.

| Code | Meaning | First checks | Typical fix path |
|---|---|---|---|
| `MAPPED_CIRCUIT_LIMIT` | Bakery is enforcing a mapped-circuit limit. | Insight license status, `journalctl -u lqosd` for requested/allowed/dropped counts. | Reduce mapped circuits immediately or update license/limits. |
| `TC_U16_OVERFLOW` | Queue/class minor IDs exceeded the Linux tc u16 range on a CPU queue. | `journalctl -u lqos_scheduler -u lqosd`, topology depth/queue distribution. | Increase queue count and/or simplify/rebalance hierarchy (for example with integration strategy or root promotion changes). |
| `TC_QDISC_CAPACITY` | Planned auto-allocated qdiscs exceed the per-interface safe budget or Bakery's conservative memory-safety preflight before apply. | Estimated per-interface qdisc counts, qdisc-kind breakdown, and memory fields in the urgent issue context, `journalctl -u lqos_scheduler -u lqosd`, `on_a_stick` and `queue_mode` config. | Reduce the planned qdisc load for this run (for example fewer circuits/devices in the test shape) before retrying; do not trust partial apply. |
| `BAKERY_MEMORY_GUARD` | A chunked Bakery full reload was stopped mid-apply because available host memory fell below the scaled safety floor. | `journalctl -u lqosd`, available/total memory in the urgent issue context, and recent Bakery apply progress. | Treat the run as failed, reduce memory pressure or queue footprint, and retry only after the host is stable. |
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
