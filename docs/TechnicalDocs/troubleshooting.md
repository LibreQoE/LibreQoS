# Troubleshooting

## Common Issues

### LibreQoS Is Running, But Traffic Not Shaping

In ispConfig.py, make sure the edge and core interfaces correspond to correctly to the edge and core. Try swapping the interfaces to see if shaping starts to work.

Make sure your services are running properly

- `lqosd.service`
- `lqos_node_manager`
- `lqos_scheduler`

Node manager and scheduler are dependent on the `lqos.service` being in a healthy, running state.

For example to check the status of lqosd, run:
```sudo systemctl status lqosd```

### lqosd not running or failed to start
At the command-line, type ```sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd``` which will provide specifics regarding why it failed to start.

### RTNETLINK answers: Invalid argument

This tends to show up when the MQ qdisc cannot be added correctly to the NIC interface. This would suggest the NIC has insufficient RX/TX queues. Please make sure you are using the [recommended NICs](../SystemRequirements/Networking.md).

### InfluxDB "Failed to update bandwidth graphs"

The scheduler (scheduler.py) runs the InfluxDB integration within a try/except statement. If it fails to update InfluxDB, it will report "Failed to update bandwidth graphs".
To find the exact cause of the failure, please run ```python3 graphInfluxDB.py``` which will provde more specific errors.

### All customer IPs are listed under Unknown IPs, rather than Shaped Devices in GUI
```
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo python3 LibreQoS.py
```

The console output from running LibreQoS.py directly provides more specific errors regarding issues with ShapedDevices.csv and network.json
Once you have identified the error and fixed ShapedDevices.csv and/or Network.json, please then run

```sudo systemctl start lqos_scheduler```
