# Sonar Integration

First, set the relevant parameters for Sonar (sonar_api_key, sonar_api_url, etc.) in `/etc/lqos.conf`.

Current behavior notes:
- `sonar_api_url` may be either the Sonar base URL or the full GraphQL endpoint. LibreQoS normalizes it to `/api/graphql` automatically.
- Current builds page through Sonar GraphQL results instead of relying on a small first-page sample.
- Paginated Sonar GraphQL requests now use a split connect/read timeout and retry transient read timeouts before failing the import.
- Emitted Sonar identities are namespaced (for example `sonar:account:<id>` and `sonar:device:<id>`) so they remain stable across overrides and downstream tooling.
- Account device discovery now preserves inventory-item IP handling and also imports Radius account IP assignments when they exist. Inventory-backed MACs are still used for AP mapping; Radius-only IPs are added as supplemental shaping devices and overlapping subnets are de-duplicated.
- Sonar `child_accounts` are also imported when they expose their own service and usable IP data. If a child account lacks its own address, LibreQoS falls back to the parent account address so the child can still be emitted as its own circuit.
- Sonar settings now support ISP-specific recurring-service fallback rates plus a recurring-service exclusion list. LibreQoS still prefers active `DATA` services first; recurring mappings are only used when an account has no usable `DATA` service.
- If Sonar returns non-JSON content or GraphQL errors, the integration now raises a more specific error message showing the endpoint and a short response preview.

To test the Sonar Integration, use

```shell
python3 integrationSonar.py
```

On the first successful run, it creates the Sonar import and shaping data LibreQoS needs for scheduled refreshes.
Built-in Sonar integrations do not write `network.json`; keep that file for DIY/manual deployments.
You can maintain a separate DIY `network.json` when Sonar is not the system you want to use for topology.
The Sonar integration refreshes its imported topology and shaping data every run.
You have the option to run integrationSonar.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_sonar = true``` in `/etc/lqos.conf`.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
