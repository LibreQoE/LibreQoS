# LibreQoS daemons

lqosd

- Manages actual XDP code. Build with Rust.

lqos_node_manager

- Runs the GUI available at http://a.b.c.d:9123

lqos_scheduler

- lqos_scheduler handles statistics and performs continuous refreshes of LibreQoS' shapers, including pulling from any enabled CRM Integrations (UISP, Splynx).
- On start: Run a full setup of queues
- Every 10 seconds: Graph bandwidth and latency stats
- Every 30 minutes: Update queues, pulling new configuration from CRM integration if enabled

## Run daemons with systemd

You can setup `lqosd`, `lqos_node_manager`, and `lqos_scheduler` as systemd services.

```shell
sudo cp /opt/libreqos/src/bin/lqos_node_manager.service.example /etc/systemd/system/lqos_node_manager.service
sudo cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
sudo cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
```

Finally, run

```shell
sudo systemctl daemon-reload
sudo systemctl enable lqosd lqos_node_manager lqos_scheduler
```

You can now point a web browser at `http://a.b.c.d:9123` (replace `a.b.c.d` with the management IP address of your shaping server) and enjoy a real-time view of your network.

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
