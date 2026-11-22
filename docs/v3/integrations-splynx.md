# Splynx Integration

**Version: LibreQoS v3.**

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/libreqos/lqos.conf`.

## New Operator Quick Start

Start with this baseline unless you have a known reason to deviate:

- `topology.compile_mode = "ap_site"` (default)
- `enable_splynx = true`

Then run one manual sync and validate outputs before enabling frequent scheduler refresh cycles.

### Topology Strategies

LibreQoS supports multiple topology strategies for Splynx integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers, no hierarchy | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Single layer: AP → Clients | Low | Lowest overhead when site-level grouping is not needed |
| `ap_site` | Two layers: Site → AP → Clients | Medium | **Default**. Best balance of structure and operator clarity |
| `full` | Complete monitoring-parent topology mapping | Highest | Preserves the richer Splynx monitoring hierarchy instead of flattening to Network Sites |

When Splynx cannot infer a circuit parent from `access_device` or router metadata, LibreQoS leaves the circuit unparented for shaping instead of attaching it to a synthetic site or AP. Fix the parent in Splynx or Topology Manager if the circuit should appear under a real site or AP.

If two Splynx monitoring hardware rows have the same title, LibreQoS appends the Splynx hardware ID to those duplicate topology node names. For example, duplicate APs named `Tower South` appear as `Tower South (1122)` and `Tower South (1980)`.

In `ap_site` mode, LibreQoS treats Splynx Network Sites as site containers and monitoring rows with `access_device = 1` as AP/access nodes.

Configure the shared topology mode in `/etc/libreqos/lqos.conf` under the `[topology]` section:

```ini
[topology]
compile_mode = "ap_site"
```

**Performance Considerations:**
- `flat` and `ap_only` strategies significantly reduce CPU load by limiting network depth
- Choose `ap_site` by default, and move to `ap_only` only when you want lower overhead and can give up site-level grouping
- Only use `full` if you require complete network topology representation and have adequate CPU resources

### Promote to Root Nodes

In v2.0, `promote_to_root` moved busy sites to the top of the tree so their
shaping ran on separate CPU cores. v3 does not need this: the bridge spreads
sites and APs across its workers itself, and splits an oversized site between
its child branches. The setting is still accepted, but it changes only the
shape of the tree, not how CPU load is spread.

### Splynx API Access

The Splynx Integration uses Basic authentication. For using this type of authentication, please make sure you enable [Unsecure access](https://splynx.docs.apiary.io/#introduction/authentication) in your Splynx API key settings. Also the Splynx API key should be granted access to the necessary permissions.

| Category       | Name                         | Permission |
|----------------|------------------------------|------------|
| Tariff Plans   | Internet                     | View       |
| FUP            | Compiler                     | View       |
| FUP            | Policies                     | View       |
| FUP            | Capped Data                  | View       |
| FUP            | CAP Tariff                   | View       |
| FUP            | FUP Limits                   | View       |
| Customers      | Customer                     | View       |
| Customers      | Customers Online             | View       |
| Customers      | Customer Internet services   | View       |
| Networking     | Routers                      | View       |
| Networking     | Router contention            | View       |
| Networking     | MikroTik                     | View       |
| Networking     | Monitoring                   | View       |
| Networking     | Network Sites                | View       |
| Networking     | IPv4 Networks                | View       |
| Networking     | IPv4 Networks IP             | View       |
| Networking     | CPE                          | View       |
| Networking     | CPE AP                       | View       |
| Networking     | IPv6 Networks                | View       |
| Networking     | IPv6 Networks IP (Addresses) | View       |
| Administration | Locations                    | View       |

To test the Splynx Integration, use

```shell
sudo /opt/libreqos/venv/bin/python integrationSplynx.py
```

On the first successful run, it creates the Splynx import and shaping files LibreQoS needs for scheduled refreshes.
Legacy `integrationSplynxBandwidths.csv` values are migrated into operator `AdjustSiteSpeed` overrides in `lqos_overrides.json`, and the original CSV is renamed to a `.backup` file.

Current builds also expose a shared Ethernet port headroom policy under `Configuration -> Data Sources -> Integration Defaults`. Integrations that can supply negotiated subscriber-facing port speed use a conservative default multiplier of `0.94` unless the operator overrides it.
Current builds also expose the shared topology compile mode under `Configuration -> Data Sources -> Integration Defaults`. Existing `splynx_integration.strategy` values are retained only as a compatibility mirror during upgrade.
Topology Manager now receives a native infrastructure topology export for Splynx imports, so real sites and APs use their resolved parent chain instead of being reconstructed from the compatibility tree. Customer-derived generated sites stay compatibility-only: they are excluded from canonical/editor topology and circuits shape under the nearest real infrastructure parent instead. Current Python-backed imports are intentionally conservative: infrastructure nodes import as fixed roots or fixed-parent branches unless the importer can provide a bounded, trustworthy set of alternative parents. When a bounded move set is available, it is limited to the node's current parent plus nearby sibling-parent alternatives under the same upstream branch, or peer root parents of the same type when the current parent is itself a root.

You have the option to run integrationSplynx.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_splynx = true``` under the ```[splynx_integration]``` section in `/etc/libreqos/lqos.conf`.
Once set, run `sudo systemctl restart libreqos-scheduler`.

## 5-Minute Validation After Splynx Changes

1. Run one integration test:
```shell
sudo /opt/libreqos/venv/bin/python integrationSplynx.py
```
2. Confirm files exist and were updated:
```shell
sudo ls -lh /opt/libreqos/state/topology/topology_import.json /opt/libreqos/state/shaping/shaping_inputs.json
```
Built-in Splynx imports refresh LibreQoS's imported topology and shaping data automatically. They do not use `network.json` or `ShapedDevices.csv` as the normal working files for this integration.
3. Confirm services are healthy:
```shell
sudo systemctl status libreqos-lqosd libreqos-scheduler
journalctl -u libreqos-scheduler --since "30 minutes ago"
```
4. Check the WebUI Scheduler view and tree depth for expected strategy behavior.

If sync looks successful but data is incomplete, recheck API permissions and strategy assumptions before changing unrelated settings.

### Splynx Overrides

Use LibreQoS Web UI overrides to change the configured rate for a Site or AP.

Open the relevant node in the tree or topology views and save the desired bandwidth there. LibreQoS will preserve that operator override across future Splynx refreshes.

Do not create or depend on legacy `integrationSplynxBandwidths*.csv` template files for new deployments. The supported workflow is the regular UI-based override system.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating LibreQoS v3](operate.md)
- [Troubleshooting](troubleshooting.md)
