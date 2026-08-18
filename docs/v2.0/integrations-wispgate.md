# WISPGate Integration

First, set the relevant parameters for WISPGate in `/etc/lqos.conf`.
There should be a section as follows:

```
[wispgate_integration]
enable_wispgate = false
wispgate_api_token = "token"
wispgate_api_url = "https://your_wispgate_url.com"
```

If the section is missing, you can add it by copying the section above.
Set the appropriate values for wispgate_api_token and wispgate_api_url, then save the file.

To test the WISPGate Integration, use

```shell
python3 integrationWISPGate.py
```

On the first successful run, it creates the WISPGate import and shaping data LibreQoS needs for scheduled refreshes.
Those files are refreshed every time the WISPGate integration runs.
Built-in integrations do not write `network.json`; keep that file for DIY/manual deployments.

You have the option to run integrationWISPGate.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_wispgate = true``` in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
