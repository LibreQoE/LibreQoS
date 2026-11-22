# VISP Integration

**Version: LibreQoS v3.**

First, set the relevant parameters for VISP in `/etc/libreqos/lqos.conf`:

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
- VISP import is GraphQL-based.
- The importer prefers fast bulk service pulls and automatically backfills missing subscriber/service data from other VISP GraphQL queries when needed.
- When VISP IRM upstream-device data is populated, the importer also builds site/upstream topology for imported subscribers instead of staying flat-only.
- `network.json` is for DIY/manual deployments; built-in integrations do not overwrite it.
- VISP auth tokens are cached in `<lqos_directory>/.visp_token_cache_*.json`.

Run a manual import with:

```bash
sudo /opt/libreqos/venv/bin/python integrationVISP.py
```

To run automatically through `libreqos-scheduler`, set:
- `[visp_integration] enable_visp = true`
- then restart scheduler:

```bash
sudo systemctl restart libreqos-scheduler
```


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating LibreQoS v3](operate.md)
- [Troubleshooting](troubleshooting.md)
