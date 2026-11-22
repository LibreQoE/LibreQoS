# Powercode Integration

**Version: LibreQoS v3.**

First, set the relevant parameters for Powercode (powercode_api_key, powercode_api_url, etc.) in `/etc/libreqos/lqos.conf`.

To test the Powercode Integration, use

```shell
sudo /opt/libreqos/venv/bin/python integrationPowercode.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can modify the network.json file manually to reflect Site/AP bandwidth limits.
ShapedDevices.csv will be overwritten every time the Powercode integration is run.
You have the option to run integrationPowercode.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_powercode = true``` in `/etc/libreqos/lqos.conf`.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating LibreQoS v3](operate.md)
- [Troubleshooting](troubleshooting.md)
