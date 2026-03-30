# UISP Integration

First, set the relevant parameters for UISP in `/etc/lqos.conf`.

## New Operator Quick Chooser

Use this section first to avoid common strategy confusion.

1. If you are new to UISP integration, start with `strategy = "ap_only"`.
2. Move to `ap_site` when you need explicit site-level aggregation.
3. Use `full` when you need full hierarchy/backhaul representation and have CPU headroom.
4. Use `flat` only when hierarchy is not needed and maximum performance is the priority.

## Router-Mode Expectations

- UISP router mode is supported, but parent/route discovery quality depends on your UISP topology data quality.
- LibreQoS focuses on shaping/queue hierarchy, not subscriber lifecycle enforcement.
- Account suspension enforcement is usually handled by your edge/BNG/auth platform. Use `suspended_strategy` only for LibreQoS-side shaping behavior.

### Topology Strategies

LibreQoS supports multiple topology strategies for UISP integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers by service plan speed | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Shapes by service plan and Access Point | Low | Good balance of performance and AP-level control |
| `ap_site` | Shapes by service plan, Access Point, and Site | Medium | Site-level aggregation with moderate complexity |
| `full` | Shapes entire network including backhauls, Sites, APs, and clients | Highest | Best after topology/overrides are validated and stable |

**Performance Note:** When using `full` strategy with large networks, consider using `promote_to_root` to distribute CPU load across multiple cores.

## 5-Minute Validation After UISP Config Changes

1. Run the integration once:
```shell
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```
2. Confirm outputs were generated/refreshed as expected:
```shell
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```
3. Confirm scheduler and shaper health:
```shell
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
```
4. Validate result in WebUI:
- Scheduler Status is healthy
- Tree/Flow views reflect expected hierarchy depth for selected strategy

If hierarchy depth or parent mapping is not what you expect, revisit `strategy`, `use_ptmp_as_parent`, `exclude_sites`, and `exception_cpes` before changing other settings.

### Promote to Root Nodes (Performance Optimization)

When using `full` topology strategy, traffic can bottleneck on a single root node/CPU core.

The `promote_to_root` feature can reduce this by promoting selected sites to root-level nodes so load is distributed.

Configuration:
1. Navigate to `Configuration -> Integrations` in WebUI.
2. In `Promote to Root Nodes`, enter one site per line.

Example:
```
Remote_Site_Alpha
Remote_Site_Beta
Datacenter_West
```

When to use:
- Multi-site networks with high-capacity remote sites.
- `full` strategy deployments showing single-core saturation.

### Suspension Handling Strategies

Configure how LibreQoS handles suspended customer accounts:

| Strategy | Description | Use Case |
|----------|-------------|----------|
| `none` | Do not handle suspensions | When suspension handling is managed elsewhere |
| `ignore` | Do not add suspended customers to network map | Reduces queue count and improves performance for networks with many suspended accounts |
| `slow` | Limit suspended customers to 0.1 Mbps | Maintains minimal connectivity for suspended accounts (e.g., payment portals) |

**Choosing a Suspension Strategy:**
- Use `none` if your edge router or another system handles suspensions
- Use `ignore` to reduce system load by not creating queues for suspended customers
- Use `slow` to maintain minimal connectivity (useful for payment portals or service messages)

### Burst

- In UISP, Download Speed and Upload Speed are configured in Mbps (for example, 100 Mbps).
- In UISP, Download Burst and Upload Burst are configured in kilobytes per second (kB/s).
- Conversion and shaping:
  - burst_mbps = kB/s × 8 / 1000
  - Download Min = Download Speed (Mbps) × commit_bandwidth_multiplier
  - Download Max = (Download Speed (Mbps) + burst_mbps) × bandwidth_overhead_factor
  - Upload Min/Max are computed the same way from Upload Speed (Mbps) and Upload Burst (kB/s)
- Example:
  - UISP values: Download Speed = 100 Mbps, Download Burst = 12,500 kB/s
  - Burst adds 12,500 × 8 / 1000 = 100 Mbps
  - Download Min = 100 × commit_bandwidth_multiplier
  - Download Max = (100 + 100) × bandwidth_overhead_factor
- Quick reference (burst kB/s → added Mbps):
  - 6,250 kB/s → +50 Mbps
  - 12,500 kB/s → +100 Mbps
  - 25,000 kB/s → +200 Mbps
- Notes:
  - Leave burst empty/null in UISP to disable burst.
  - If suspended_strategy is set to slow, both Min and Max are set to 0.1 Mbps.

### Configuration Example

```ini
[uisp_integration]
# Core Settings
enable_uisp = true
token = "your-api-token-here"
url = "https://uisp.your_domain.com"
site = "Root_Site_Name"  # Root site for topology perspective

# Topology Strategy (see table above)
strategy = "ap_only"  # Recommended starting point for new UISP deployments

# Suspension Handling (see table above)
suspended_strategy = "none"

# Capacity Adjustments
# UISP's reported AP capacities can be optimistic
airmax_capacity = 0.65  # Use 65% of reported AirMax capacity
ltu_capacity = 0.95     # Use 95% of reported LTU capacity

# Site Management
exclude_sites = []  # Sites to exclude, e.g., ["Test_Site", "Lab_Site"]
use_ptmp_as_parent = true  # For sites branched off PtMP Access Points

# Bandwidth Adjustments
bandwidth_overhead_factor = 1.15  # Give customers 15% above plan speed
commit_bandwidth_multiplier = 0.98  # Set minimum to 98% of maximum (CIR)

# Advanced Options
ipv6_with_mikrotik = false  # Enable if using DHCPv6 with MikroTik
always_overwrite_network_json = true  # Recommended when using UISP integration in production
exception_cpes = []  # CPE exceptions in ["cpe:parent"] format
squash_sites = []  # Optional: sites to squash
enable_squashing = false  # Optional: enable AP/single-entry squashing logic
do_not_squash_sites = []  # Optional: never squash these sites
ignore_calculated_capacity = false  # Optional: keep configured capacities even if calculated differs
insecure_ssl = false  # Optional: ignore UISP TLS certificate validation
```

### UISP Advanced/Operational Options

The following UISP options are available in current builds and WebUI (Node Manager) config editors:

- `exclude_sites`: list of site names to exclude from import.
- `exception_cpes`: list of `cpe:parent` overrides for ambiguous parent assignment.
- `squash_sites`: optional list of sites to squash in full strategy workflows.
- `enable_squashing`: enables additional squashing behavior where supported.
- `do_not_squash_sites`: explicit exclusions from squashing.
- `use_ptmp_as_parent`: prefer PtMP AP as parent for relevant topology paths.
- `ignore_calculated_capacity`: prefer configured capacities instead of integration-calculated values.
- `insecure_ssl`: disables TLS certificate verification for UISP API calls.

Recommended use:
1. Keep `insecure_ssl = false` unless you have a known internal PKI/self-signed requirement.
2. Use `exclude_sites` and `do_not_squash_sites` first for safer topology changes.
3. Apply `squash_sites`/`enable_squashing` incrementally and validate queue placement after each change.

On the first successful run, the integration creates `network.json` and `ShapedDevices.csv`.
If a `network.json` file exists, it is only overwritten when `always_overwrite_network_json = true`.

ShapedDevices.csv will be overwritten every time the UISP integration is run.

When UISP client sites share the same name, LibreQoS now tries to disambiguate the generated circuit/site display names with a human-friendly suffix such as the first street-address segment, falling back to service name and then a short ID only when needed. Stable circuit identity still comes from the UISP site/service ID, not the display name.

For integration-driven deployments, keep `always_overwrite_network_json = true` so topology stays aligned with UISP on each refresh cycle.

You have the option to run `uisp_integration` automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`. Once set, run `sudo systemctl restart lqos_scheduler`.

### UISP Overrides

You can also modify the following files to more accurately reflect your network:
- integrationUISPbandwidths.csv
- integrationUISProutes.csv

Tree-page `Operator Override` edits are separate from these legacy UISP files. Current builds write those operator-owned node rate changes to `lqos_overrides.json` and do not rewrite `integrationUISPbandwidths.csv`.

Each of the files above have templates available in the `/opt/libreqos/src` folder. If you don't find them there, you can navigate [here](https://github.com/LibreQoE/LibreQoS/tree/develop/src). To utilize the template, copy the file (removing the `.template` part of the filename) and set the appropriate information inside each file.
For example, if you want to change the set bandwidth for a site, you would do:
```
sudo cp /opt/libreqos/src/integrationUISPbandwidths.template.csv /opt/libreqos/src/integrationUISPbandwidths.csv
```
And edit the CSV using LibreOffice or your preferred CSV editor.

To avoid conflicting sources of truth, prefer one durable override path per node: either the legacy UISP CSV workflow or the operator override layer.

#### UISP Route Overrides

The default cost between nodes is typically 10. The integration creates a dot graph file `/opt/libreqos/src/graph.dot` which can be rendered using [Graphviz](https://dreampuf.github.io/GraphvizOnline/). This renders a map with the associated costs of all links.

![image](https://github.com/user-attachments/assets/4edba4b5-c377-4659-8798-dfc40d50c859)

Say you have Site 1, Site 2, and Site 3.
A backup path exists between Site 1 and Site 3, but is not the preferred path.
Your preference is Site 1 > Site 2 > Site 3, but the integration by default connects Site 1 > Site 3 directly.

To fix this, add a cost above the default for the path between Site 1 and Site 3.
```
Site 1, Site 3, 100
Site 3, Site 1, 100
```
With this, data will flow Site 1 > Site 2 > Site 3.

To make the change, perform a reload of the integration with ```sudo systemctl restart lqos_scheduler```.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
