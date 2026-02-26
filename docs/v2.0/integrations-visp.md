# VISP Integration

First, set the relevant parameters for VISP in `/etc/lqos.conf`:

```ini
[visp_integration]
enable_visp = true
client_id = "your-client-id"
client_secret = "your-client-secret"
username = "appuser-username"
password = "appuser-password"
# Optional: leave unset/blank to auto-select first ISP ID returned by token payload
# isp_id = 0
timeout_secs = 20
# Optional: used for online session enrichment
# online_users_domain = ""
```

Notes:
- VISP import is GraphQL-based and currently defaults to a flat topology strategy.
- The integration writes `ShapedDevices.csv` every run.
- `network.json` is only overwritten when `always_overwrite_network_json = true` (under `[integration_common]`).
- Recommended: keep `always_overwrite_network_json = true` for integration-driven deployments so topology stays aligned with VISP syncs.
- VISP auth tokens are cached in `<lqos_directory>/.visp_token_cache_*.json`.

Run a manual import with:

```bash
python3 integrationVISP.py
```

To run automatically through `lqos_scheduler`, set:
- `[visp_integration] enable_visp = true`
- then restart scheduler:

```bash
sudo systemctl restart lqos_scheduler
```


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
