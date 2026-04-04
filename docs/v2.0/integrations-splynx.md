# Splynx Integration

> **⚠️ Breaking Change Notice**: Prior to v1.5-RC-2, the default Splynx strategy was `full`. Starting with v1.5-RC-2, the default strategy is `ap_only` for improved CPU performance. If you require the previous behavior, explicitly set `strategy = "full"` in your Splynx configuration section.

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

## New Operator Quick Start

Start with this baseline unless you have a known reason to deviate:

- `strategy = "ap_only"` (default, lowest confusion)
- `enable_splynx = true`
- `always_overwrite_network_json = true` for integration-driven deployments

Then run one manual sync and validate outputs before enabling frequent scheduler refresh cycles.

Overwrite policy:
- Recommended: keep `always_overwrite_network_json = true` so topology stays aligned with Splynx on each refresh cycle.

### Topology Strategies

LibreQoS supports multiple topology strategies for Splynx integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers, no hierarchy | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Single layer: AP → Clients | Low | **Default**. Best balance of performance and structure |
| `ap_site` | Two layers: Site → AP → Clients | Medium | Site-level aggregation with moderate complexity |
| `full` | Complete topology mapping | Highest | Full network hierarchy representation |

Configure the strategy in `/etc/lqos.conf` under the `[splynx_integration]` section:

```ini
[splynx_integration]
# ... other splynx settings ...
strategy = "ap_only"
```

**Performance Considerations:**
- `flat` and `ap_only` strategies significantly reduce CPU load by limiting network depth
- Choose `ap_only` for most deployments unless you need site-level traffic aggregation
- Only use `full` if you require complete network topology representation and have adequate CPU resources

### Promote to Root Nodes (Performance Optimization)

When using `full` topology strategy, you may encounter CPU performance bottlenecks where all traffic flows through a single root site, limiting throughput to what one CPU core can handle.

The **promote_to_root** feature solves this by promoting specific sites to root-level nodes, distributing traffic shaping across multiple CPU cores.

**Configuration:**
1. Navigate to Integration → Common in the WebUI
2. In the "Promote to Root Nodes" field, enter one site name per line:
```
Remote_Site_Alpha
Remote_Site_Beta
Datacenter_West
```

**Benefits:**
- Eliminates single-CPU bottleneck for networks with remote sites
- Distributes traffic shaping across multiple CPU cores
- Improves overall network performance for large topologies
- Works with both Splynx and UISP integrations

**When to Use:**
- Networks with multiple high-capacity remote sites
- When using `full` topology strategy and experiencing CPU limitations
- Large networks where root site traffic exceeds single-core capacity

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
python3 integrationSplynx.py
```

On the first successful run, it creates `ShapedDevices.csv` and `network.json`.
ShapedDevices.csv will be overwritten every time the Splynx integration is run.

Current builds also expose a shared Ethernet port headroom policy under `Configuration -> Integrations -> Integration Defaults`. Integrations that can supply negotiated subscriber-facing port speed use a conservative default multiplier of `0.94` unless the operator overrides it.

You have the option to run integrationSplynx.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_splynx = true``` under the ```[splynx_integration]``` section in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.

## 5-Minute Validation After Splynx Changes

1. Run one integration test:
```shell
python3 integrationSplynx.py
```
2. Confirm files exist and were updated:
```shell
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```
3. Confirm services are healthy:
```shell
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
```
4. Check WebUI Scheduler Status and tree depth for expected strategy behavior.

If sync looks successful but data is incomplete, recheck API permissions and strategy assumptions before changing unrelated settings.

### Splynx Overrides

You can also modify the the file `integrationSplynxBandwidths.csv` to override the default bandwidths for each Node (Site, AP).

Tree-page `Operator Override` edits are separate from this legacy Splynx file. Current builds write those operator-owned node rate changes to `lqos_overrides.json` and do not rewrite `integrationSplynxBandwidths.csv`.

A template is available in the `/opt/libreqos/src` folder. To utilize the template, copy the file `integrationSplynxBandwidths.template.csv` (removing the `.template` part of the filename) and set the appropriate information inside each file. For example, if you want to change the set bandwidth for a site, you would do:
```
sudo cp /opt/libreqos/src/integrationSplynxBandwidths.template.csv /opt/libreqos/src/integrationSplynxBandwidths.csv
```
And edit the CSV using LibreOffice or your preferred CSV editor.

To avoid conflicting sources of truth, prefer one durable override path per node: either the legacy Splynx CSV workflow or the operator override layer.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
