# LibreQoS daemons

lqosd

- Manages actual XDP code. Build with Rust.
- Runs the GUI available at http://a.b.c.d:9123

lqos_scheduler

- lqos_scheduler handles statistics and performs continuous refreshes of LibreQoS' shapers, including pulling from any enabled CRM Integrations (UISP, Splynx).
- On start: Run a full setup of queues
- Every 30 minutes: Update queues, pulling new configuration from CRM integration if enabled
  - Minute interval is adjustable with the setting `queue_refresh_interval_mins` in `/etc/lqos.conf`.

## Debugging lqos_scheduler

In the background, lqos_scheduler runs scheduler.py, which in turn runs LibreQoS.py

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
