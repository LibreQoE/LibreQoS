# UISP Integration

First, set the relevant parameters for UISP in `/etc/lqos.conf`.

## New Operator Quick Chooser

Use this section first to avoid common topology-mode confusion.

1. If you are new to UISP integration, start with `topology.compile_mode = "ap_site"` under `Integration - Common`.
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
ls -lh /opt/libreqos/src/topology_import.json /opt/libreqos/src/shaping_inputs.json
```
Built-in UISP imports refresh LibreQoS's imported topology and shaping data automatically. They do not use `network.json`, `ShapedDevices.csv`, or a standalone `circuit_anchors.json` as the normal working files for this integration.
3. Confirm scheduler and shaper health:
```shell
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
```
4. Validate result in WebUI:
- Scheduler Status is healthy
- Tree/Flow views reflect expected hierarchy depth for the selected compile mode

If hierarchy depth or parent mapping is not what you expect, revisit `topology.compile_mode`, `use_ptmp_as_parent`, `exclude_sites`, and `exception_cpes` before changing other settings.

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
[topology]
# Shared topology compile mode (see table above)
compile_mode = "ap_site"  # Recommended starting point for new UISP deployments

[uisp_integration]
# Core Settings
enable_uisp = true
token = "your-api-token-here"
url = "https://uisp.your_domain.com"
site = "Root_Site_Name"  # Root site for topology perspective

# Suspension Handling (see table above)
suspended_strategy = "none"

# Capacity Adjustments
# Current defaults normalize both AP capacity multipliers to 1.0
airmax_capacity = 1.0  # Use 100% of reported AirMax capacity
airmax_flexible_frame_download_ratio = 0.8  # Fallback split for AirMax flexible framing when UISP does not expose dlRatio
ltu_capacity = 1.0      # Use 100% of reported LTU capacity
infrastructure_transport_caps_enabled = true  # Automatically cap radio capacity to active/model Ethernet transport ceilings

# Site Management
exclude_sites = []  # Sites to exclude, e.g., ["Test_Site", "Lab_Site"]
use_ptmp_as_parent = true  # For sites branched off PtMP Access Points

# Bandwidth Adjustments
bandwidth_overhead_factor = 1.15  # Give customers 15% above plan speed
commit_bandwidth_multiplier = 0.98  # Set minimum to 98% of maximum (CIR)

# Advanced Options
ipv6_with_mikrotik = false  # Enable if using DHCPv6 with MikroTik
# `network.json` is for DIY/manual deployments; built-in integrations do not write it
exception_cpes = []  # CPE exceptions in ["cpe:parent"] format
squash_sites = []  # Optional: sites to squash
do_not_squash_sites = []  # Optional: keep these site names unsquashed in the runtime/export tree
ignore_calculated_capacity = false  # Optional: keep configured capacities even if calculated differs
insecure_ssl = false  # Optional: ignore UISP TLS certificate validation
```

### UISP Advanced/Operational Options

The following UISP options are available in current builds and WebUI (Node Manager) config editors:

- `exclude_sites`: list of site names to exclude from import.
- `exception_cpes`: list of `cpe:parent` overrides for ambiguous parent assignment.
- `squash_sites`: optional list of sites to squash in full strategy workflows.
- `do_not_squash_sites`: explicit site-name exclusions from runtime/export squashing.
- `use_ptmp_as_parent`: prefer PtMP AP as parent for relevant topology paths.
- `ignore_calculated_capacity`: prefer configured capacities instead of integration-calculated values.
- `infrastructure_transport_caps_enabled`: automatically cap UISP radio/device attachment rates to observed or known transport-port ceilings before they enter topology/export.
- `insecure_ssl`: disables TLS certificate verification for UISP API calls.
- `airmax_flexible_frame_download_ratio`: when UISP reports aggregate AirMax AP capacity for flexible framing and the live `dlRatio` is absent, LibreQoS uses this fallback download share. `0.8` means 80/20 download/upload.

Topology Manager attachment-health probes use UISP-reported management IPs for the selected attachment pair. Current builds no longer prune those probe IPs through shaping `allow_subnets`; the shaping address allowlist still applies to generated subscriber/device shaping data, but not to management-plane topology probe targets.

Topology Manager attachment-rate overrides stay disabled for UISP attachments whose rates come directly from dynamic radio-capacity telemetry, such as radios where UISP is actively reporting live directional capacity. Static UISP attachments, black-box/fallback cases, and manual attachment groups remain eligible for attachment-scoped rate overrides.

Current UISP builds also classify attachment feed roles for Topology Manager and runtime export. Typical roles are `PtP Backhaul`, `PtMP Uplink`, and `Wired Uplink`. Runtime/export squashing only collapses effective backhaul-style roles; PtMP access/uplink APs stay visible in `tree.html`.

Shared integration defaults also include Ethernet port limiting. When UISP can detect negotiated subscriber-facing Ethernet speed, current builds apply a conservative default multiplier of `0.94` unless the operator overrides it in `Configuration -> Integrations -> Integration Defaults`.

Current UISP builds also use that same conservative multiplier for infrastructure attachment transport caps when `infrastructure_transport_caps_enabled = true`. LibreQoS prefers the highest active transport-looking Ethernet/SFP interface reported by UISP for those infrastructure caps, with exact model fallbacks for known hardware ceilings such as AF60-LR.

Current builds scope this flexible-frame handling narrowly to devices where UISP reports `identification.type == "airMax"` and `identification.role == "ap"`. Those AirMax APs use `theoreticalTotalCapacity` only as a flexible-framing hint. The actual shaping rate comes from aggregate `totalCapacity` when UISP provides it, otherwise from the stronger directional capacity, and the split still prefers the live wireless `dlRatio` when UISP provides one.

Recommended use:
1. Keep `insecure_ssl = false` unless you have a known internal PKI/self-signed requirement.
2. Use `exclude_sites` and `do_not_squash_sites` first for safer topology changes.
3. UISP runtime/export squashing is always enabled after Topology Manager. Use `do_not_squash_sites` only when a specific site path must remain unsquashed.

Legacy note:
- Existing `enable_squashing` values in `/etc/lqos.conf` are ignored for backward compatibility.
- Existing `uisp_integration.strategy` values are retained only as a compatibility mirror. Current builds read the shared `topology.compile_mode` first.

On the first successful run, the integration creates the UISP import and shaping files LibreQoS needs for scheduled refreshes.

If UISP exposes a site and an AP with the same visible name in the same topology, current builds keep them distinct so the site branch is not dropped from runtime views.

When UISP client sites share the same name, LibreQoS now tries to disambiguate the generated circuit/site display names with a human-friendly suffix such as the first street-address segment, falling back to service name and then a short ID only when needed. Stable circuit identity still comes from the UISP site/service ID, not the display name.

You have the option to run `uisp_integration` automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`. Once set, run `sudo systemctl restart lqos_scheduler`.

### UISP Overrides

You can also use the following override inputs to more accurately reflect your network:
- Tree-page `Rate Override` edits, stored as `AdjustSiteSpeed` entries in `lqos_overrides.json`
- Tree-page `Topology Override` edits for supported UISP `full` nodes, also stored in `lqos_overrides.json`
- integrationUISPbandwidths.csv as a legacy compatibility input only

Current UISP builds auto-migrate a legacy `integrationUISPbandwidths.csv` into `AdjustSiteSpeed` overrides on the next integration run when no newer rate overrides exist yet. If newer rate overrides already exist, the CSV is ignored and a warning is logged so only one override method stays active.
Deprecated legacy `uisp.bandwidth_overrides` JSON entries are ignored. The supported long-term bandwidth override path is `AdjustSiteSpeed` in `lqos_overrides.json`.
Current UISP builds ignore legacy `uisp.route_overrides` entries in `lqos_overrides.json` and legacy `integrationUISProutes.csv` files. If either is present, LibreQoS logs a warning and uses detected topology plus Topology Manager overrides instead.

UISP `full` strategy builds also expose tree-page `Topology Override` editing for supported nodes. Current WebUI support is `Pinned Parent` only.

Each of the files above have templates available in the `/opt/libreqos/src` folder. If you don't find them there, you can navigate [here](https://github.com/LibreQoE/LibreQoS/tree/develop/src).

For new bandwidth changes, use the operator override layer. `integrationUISPbandwidths.csv` remains a compatibility input for one-time migration into `AdjustSiteSpeed`, not the preferred long-term workflow.

For path intent, use Topology Manager parent selection and attachment preference. That is now the supported replacement for older UISP route-cost overrides.


## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)
