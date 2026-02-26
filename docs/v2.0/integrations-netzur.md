# Netzur Integration

Netzur deployments expose subscriber and zone inventories via a REST endpoint secured with a Bearer token. Configure `/etc/lqos.conf` as follows:

```ini
[netzur_integration]
enable_netzur = true
api_key = "your-netzur-token"
api_url = "https://netzur.example.com/api/libreqos"
timeout_secs = 60
use_mikrotik_ipv6 = false
```

- `enable_netzur` toggles automatic imports by `lqos_scheduler`.
- `api_key` is the Bearer token generated inside Netzur.
- `api_url` must return JSON containing `zones` (mapped to sites) and `customers` (mapped to client circuits and devices).
- `timeout_secs` overrides the default HTTP timeout (60 seconds) when the API is slow.
- `use_mikrotik_ipv6` enriches subscriber devices with IPv6 prefixes discovered via `mikrotikDHCPRouterList.csv`.

Run a manual import with:

```bash
python3 integrationNetzur.py
```

The integration regenerates `ShapedDevices.csv` and, unless `always_overwrite_network_json` is disabled, updates `network.json`. Adjust the Integration → Common settings if you need to preserve an existing network map between Netzur syncs.

Overwrite policy:
- Recommended: keep `always_overwrite_network_json = true` for integration-driven deployments so topology stays aligned with Netzur syncs.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
