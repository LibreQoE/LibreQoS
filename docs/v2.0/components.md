# LibreQoS Software Components

## Systemd Services

```{mermaid}
flowchart LR
    A[CRM/NMS Integrations] --> B[lqos_scheduler]
    C[network.json + ShapedDevices.csv] --> B
    D[lqos_overrides.json] --> B
    B --> E[Queue/shaping plan refresh]
    E --> F[lqosd]
    F --> G[XDP/TC shaping runtime]
    F --> H[WebUI :9123]
    B --> I[Scheduler Status / Urgent Issues]
    F --> I
```

### lqosd

- Manages actual XDP code.
- Coded in Rust.
- Runs the GUI available at http://a.b.c.d:9123
- Hosts WebUI pages such as:
  - Site Map
  - Flow Globe
  - Network Tree Overview
  - ASN Analysis
  - CPU Tree / CPU Weights
  - Configuration editors for integrations (UISP, Splynx, Netzur, VISP, etc.)

### lqos_scheduler

- lqos_scheduler performs continuous refreshes of LibreQoS' shapers, including pulling from any enabled CRM Integrations (UISP, Splynx, Netzur).
- Actions:
  - On start: Run a full setup of queues
    - Current builds wait briefly for `lqosd` to finish binding the local bus before the first scheduler run.
    - Current builds also wait for `lqos_topology` to publish `topology_runtime_status.json` with `ready: true` for the exact current `source_generation`, rather than inferring readiness from file mtimes.
  - Every X minutes: Update queues, pulling new configuration from CRM integration, if enabled.
    - The default minute interval is 30, so the refresh occurs every 30 minutes by default.
    - The minute interval is adjustable with the setting `queue_refresh_interval_mins` in `/etc/lqos.conf`.
  - Current packages build `liblqos_python.so` with PyO3 `abi3` using a Python 3.10 floor, so the shipped extension is intended to remain import-compatible across newer CPython 3.x minor versions supported by PyO3 rather than matching only the build host's exact Python minor.

### lqos_topology runtime contract

- `lqos_topology` continuously builds runtime-effective topology artifacts from current source inputs and attachment health.
- Current builds keep two distinct topology views:
  - `compatibility_network_json` remains the local compatibility tree used as the base for `network.effective.json`
  - Insight topology submission derives a separate logical-parent tree from canonical topology state so sites are grouped by logical site hierarchy rather than immediate attachment hops
  - When possible, the Insight-only logical tree preserves existing `network.json` export names for node identity compatibility instead of renaming nodes to raw canonical labels
- After a successful publish, it writes `/opt/libreqos/src/topology_runtime_status.json` with:
  - `source_generation`
  - `ready`
  - `generated_unix`
  - artifact paths for `topology_effective_state.json`, `network.effective.json`, and `shaping_inputs.json`
  - `error` when the current generation failed
- `lqos_scheduler` only calls `refreshShapers()` when that status file reports `ready: true` for the exact current generation of `network.json`, `ShapedDevices.csv`, `circuit_anchors.json` when present, and the active topology source state.
- If the runtime status is missing, stale, or failed for the current generation, scheduler stays alive in degraded mode and retries automatically on later refreshes.

### Checking service status

```
sudo systemctl status lqosd lqos_scheduler
```

If the status of one of the two services shows 'failed', examine why using journalctl, which shows the full status of the service. For example, if lqosd failed, you would run:
```
sudo journalctl -u lqosd -b
```
Press the End key on the keyboard to take you to the bottom of the log to see the latest updates to that log.

Lqosd will provide specific reasons it failed, such as an interface not being up, an interface lacking multi-queue, or other concerns.

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

If an integration fails during a scheduler run, current builds keep the scheduler alive, surface a shortened output preview in scheduler status/error reporting, and save the full captured output to a timestamped `/tmp/lqos_scheduler_<integration>_*.log` file.

Once you have any errors eliminated, restart lqos_scheduler with

```shell
sudo systemctl start lqos_scheduler
```

### Troubleshooting Service Components
Please see [Troubleshooting](troubleshooting.md).

## WebUI privacy mode

WebUI (Node Manager) includes a client-side redaction mode for demos/screenshots:
- Toggle using the mask icon in the top navigation.
- Redaction preference is saved in browser local storage.
- This masks/redacts visible data in the browser UI; it does not modify source data files.

For public screenshots, enable redaction before capture.

## Urgent issue channel

WebUI (Node Manager) includes an urgent issue channel for high-priority events (for example, mapped-circuit limit enforcement and related operational warnings).

- Urgent issues appear in the top navigation indicator.
- They can be reviewed and acknowledged from the urgent issues modal.
- Use this as an at-a-glance operational signal; confirm details in `journalctl -u lqosd`.

## Scheduler status indicator

WebUI (Node Manager) includes scheduler status visibility for operator awareness.

- Use scheduler status as a quick health signal for recurring refresh jobs.
- During startup and scheduled refresh phases, the sidebar indicator reports coarse progress and the current scheduler phase instead of showing a blind spinner.
- If scheduler is not healthy, validate `lqosd` and `lqos_scheduler` service state first.
- Confirm details with:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - `journalctl -u lqosd --since "30 minutes ago"`
