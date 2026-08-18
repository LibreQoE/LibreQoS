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
- `use_mikrotik_ipv6` enriches subscriber devices with IPv6 prefixes discovered via `/etc/libreqos/mikrotik_ipv6.toml`. LibreQoS matches IPv4 and IPv6 by MAC address using MikroTik ARP, DHCPv4, DHCPv6, and IPv6 neighbor data.

When `use_mikrotik_ipv6` is enabled, create `/etc/libreqos/mikrotik_ipv6.toml`. Add one `[[router]]` block for each MikroTik router LibreQoS should query:

```toml
version = 1

[[router]]
name = "core-1"
host = "100.64.0.1"
port = 8728
username = "libreqos"
password = "secret"
use_ssl = false
plaintext_login = true

[[router]]
name = "core-2"
host = "100.64.0.2"
port = 8728
username = "libreqos"
password = "secret"
use_ssl = false
plaintext_login = true
```

Use additional `[[router]]` blocks for additional routers. `port`, `use_ssl`, and `plaintext_login` can be omitted when the defaults are correct: port `8728`, SSL disabled, and plaintext login enabled.

Run a manual import with:

```bash
python3 integrationNetzur.py
```

The integration regenerates `ShapedDevices.csv` for its legacy DIY-style path, but built-in integrations do not write `network.json`. Keep `network.json` for DIY/manual deployments.

For integration-driven workflows, validate the import in WebUI and through the current topology/shaping files rather than editing `network.json`.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
