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
- If scheduler is not healthy, validate `lqosd` and `lqos_scheduler` service state first.
- Confirm details with:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - `journalctl -u lqosd --since "30 minutes ago"`
