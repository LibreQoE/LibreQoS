# CRM/NMS Integrations

This page preserves detailed legacy integration reference material.
Canonical, task-oriented guidance lives in the per-integration pages linked from [CRM/NMS Integrations](integrations.md).

  * [Splynx Integration](#splynx-integration)
    + [Topology Strategies](#topology-strategies)
    + [Promote to Root Nodes (Performance Optimization)](#promote-to-root-nodes-performance-optimization)
    + [Splynx API Access](#splynx-api-access)
    + [Splynx Overrides](#splynx-overrides)
  * [Netzur Integration](#netzur-integration)
  * [VISP Integration](#visp-integration)
  * [UISP Integration](#uisp-integration)
    + [Topology Strategies](#topology-strategies-1)
    + [Suspension Handling Strategies](#suspension-handling-strategies)
    + [Burst](#burst)
    + [Configuration Example](#configuration-example)
    + [UISP Overrides](#uisp-overrides)
  * [WISPGate Integration](#wispgate-integration)
  * [Powercode Integration](#powercode-integration)
  * [Sonar Integration](#sonar-integration)

Most operators use this built-in integration path.
If you use your own scripts as the source of truth for `network.json` and `ShapedDevices.csv`, start with [Operating Modes and Source of Truth](operating-modes.md).

## What these integrations do

These integrations synchronize with your CRM/NMS system to produce integration-owned topology and shaping artifacts for LibreQoS.

Important behavior when integrations are enabled:
- Built-in integrations publish integration-owned topology and shaping artifacts.
- `network.json` remains a DIY/manual ingress file and is not written by built-in integrations.
- Manual edits may be overwritten on the next refresh cycle.

## Splynx Integration

> **⚠️ Breaking Change Notice**: Prior to v1.5-RC-2, the default Splynx strategy was `full`. The default shared topology mode is now `ap_site` as the best balance of structure and operator clarity. If you require the previous behavior, explicitly set `strategy = "full"` in your Splynx configuration section.

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

### Topology Strategies

LibreQoS supports multiple topology strategies for Splynx integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers, no hierarchy | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Single layer: AP → Clients | Low | Lowest overhead when site-level grouping is not needed |
| `ap_site` | Two layers: Site → AP → Clients | Medium | **Default**. Best balance of structure and operator clarity |
| `full` | Complete topology mapping | Highest | Full network hierarchy representation |

Configure the strategy in `/etc/lqos.conf` under the `[splynx_integration]` section:

```ini
[splynx_integration]
# ... other splynx settings ...
strategy = "ap_site"
```

**Performance Considerations:**
- `flat` and `ap_only` strategies significantly reduce CPU load by limiting network depth
- Choose `ap_site` by default, and move to `ap_only` only when you want lower overhead and can give up site-level grouping
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

On the first successful run, it will create integration-owned shaping data.
Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

You have the option to run integrationSplynx.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_splynx = true``` under the ```[splynx_integration]``` section in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.

### Splynx Overrides

Use LibreQoS Web UI overrides to change the configured rate for a Site or AP.

Open the relevant node in the tree or topology views and save the desired bandwidth there. LibreQoS will preserve that operator override across future Splynx refreshes.

Do not create or depend on legacy `integrationSplynxBandwidths*.csv` template files for new deployments. The supported workflow is the regular UI-based override system.

## Netzur Integration

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
- `use_mikrotik_ipv6` enriches subscriber devices with IPv6 prefixes discovered via `/etc/libreqos/mikrotik_ipv6.toml`.

Run a manual import with:

```bash
python3 integrationNetzur.py
```

The integration regenerates `ShapedDevices.csv` for its legacy DIY-style path, but built-in integrations no longer write `network.json`. Keep DIY `network.json` operator-owned.

## VISP Integration

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
- `network.json` remains a DIY/manual ingress file; built-in integrations do not overwrite it.
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

## UISP Integration

First, set the relevant parameters for UISP in `/etc/lqos.conf`.

### Topology Strategies

LibreQoS supports multiple topology strategies for UISP integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers by service plan speed | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Shapes by service plan and Access Point | Low | Good balance of performance and AP-level control |
| `ap_site` | Shapes by service plan, Access Point, and Site | Medium | Site-level aggregation with moderate complexity |
| `full` | Shapes entire network including backhauls, Sites, APs, and clients | Highest | Best after topology/overrides are validated and stable |

**Choosing the Right Strategy:**
- Start with `ap_site` for new deployments and initial validation
- Move to `ap_site` when you need site-level control but not backhaul shaping
- Move to `full` after topology quality and overrides are validated, and CPU headroom is confirmed
- Use `flat` only when maximum performance is critical and you don't need any hierarchy

**Performance Note:** When using `full` strategy with large networks, consider using the **promote_to_root** feature (see [Promote to Root Nodes](#promote-to-root-nodes-performance-optimization) above) to distribute CPU load across multiple cores.

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
strategy = "ap_site"  # Recommended starting point for new UISP deployments

# Suspension Handling (see table above)
suspended_strategy = "none"

# Capacity Adjustments
# UISP's reported AP capacities can be optimistic
airmax_capacity = 0.8  # Use 80% of reported AirMax capacity on new installs
airmax_flexible_frame_download_ratio = 0.8  # Fallback split for AirMax flexible framing when UISP does not expose dlRatio
ltu_capacity = 1.0      # Use 100% of reported LTU capacity on new installs
infrastructure_transport_caps_enabled = true  # Automatically cap radio capacity to active/model Ethernet transport ceilings

# Site Management
exclude_sites = []  # Sites to exclude, e.g., ["Test_Site", "Lab_Site"]
use_ptmp_as_parent = true  # For sites branched off PtMP Access Points

# Bandwidth Adjustments
bandwidth_overhead_factor = 1.15  # Give customers 15% above plan speed
commit_bandwidth_multiplier = 0.98  # Set minimum to 98% of maximum (CIR)

# Advanced Options
ipv6_with_mikrotik = false  # Enable if using DHCPv6 with MikroTik
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
- `insecure_ssl`: disables TLS certificate verification for UISP API calls.
- `airmax_flexible_frame_download_ratio`: when UISP reports aggregate AirMax AP capacity for flexible framing and the live `dlRatio` is absent, LibreQoS uses this fallback download share. `0.8` means 80/20 download/upload.

Topology Manager attachment-health probes use UISP-reported management IPs for the selected attachment pair. Current builds no longer prune those probe IPs through shaping `allow_subnets`; the shaping address allowlist still applies to generated subscriber/device shaping data, but not to management-plane topology probe targets.

Current builds scope this flexible-frame handling narrowly to devices where UISP reports `identification.type == "airMax"` and `identification.role == "ap"`. Those AirMax APs use `theoreticalTotalCapacity` only as a flexible-framing hint. The actual shaping rate comes from aggregate `totalCapacity` when UISP provides it, otherwise from the stronger directional capacity, and the split still prefers the live wireless `dlRatio` when UISP provides one.

Recommended use:
1. Keep `insecure_ssl = false` unless you have a known internal PKI/self-signed requirement.
2. Use `exclude_sites` and `do_not_squash_sites` first for safer topology changes.
3. UISP runtime/export squashing is always enabled after Topology Manager. Use `do_not_squash_sites` only when a specific site path must remain unsquashed.

Legacy note:
- Existing `enable_squashing` values in `/etc/lqos.conf` are ignored for backward compatibility.

To test the UISP Integration, use

```shell
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```

On the first successful run, it will create a network.json and ShapedDevices.csv file.
Built-in integrations do not overwrite an existing `network.json`; keep DIY `network.json` operator-owned.

ShapedDevices.csv will be overwritten every time the UISP integration is run.

If UISP exposes a site and an AP with the same visible name in the same topology, current builds keep the site name stable in `network.json` and disambiguate the AP name during export so the site branch is not dropped from the tree.

Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

You have the option to run `uisp_integration` automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`. Once set, run `sudo systemctl restart lqos_scheduler`.

### UISP Overrides

You can also use the following override inputs to more accurately reflect your network:
- `Rate Override` for node bandwidth changes stored as operator `AdjustSiteSpeed` entries in `lqos_overrides.json`
- `Topology Override` for UISP `full` parent-selection corrections stored in `lqos_overrides.json`
- integrationUISPbandwidths.csv as a legacy compatibility input only

Current builds apply `Topology Override` before final `network.json` / `ShapedDevices.csv` emission. Current WebUI support is `Pinned Parent` only, forcing one detected immediate upstream parent.

Current UISP builds also auto-migrate a legacy `integrationUISPbandwidths.csv` into operator `AdjustSiteSpeed` overrides on the next integration run when no operator rate overrides exist yet. If operator rate overrides already exist, the CSV is ignored.
Deprecated legacy `uisp.bandwidth_overrides` JSON entries are ignored. The authoritative bandwidth override path is operator `AdjustSiteSpeed` in `lqos_overrides.json`.
Current UISP builds ignore legacy `uisp.route_overrides` entries in `lqos_overrides.json` and legacy `integrationUISProutes.csv` files. If either is present, LibreQoS logs a warning and uses detected topology plus Topology Manager overrides instead.

Each of the files above have templates available in the `/opt/libreqos/src` folder. If you don't find them there, you can navigate [here](https://github.com/LibreQoE/LibreQoS/tree/develop/src).

For path intent, use Topology Manager parent selection and attachment preference. That is now the supported replacement for older UISP route-cost overrides.

## WISPGate Integration

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

On the first successful run, it will create a ShapedDevices.csv file and network.json.
ShapedDevices.csv will be overwritten every time the WISPGate integration is run.

Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

You have the option to run integrationWISPGate.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_wispgate = true``` in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.

## Powercode Integration

First, set the relevant parameters for Powercode (powercode_api_key, powercode_api_url, etc.) in `/etc/lqos.conf`.

To test the Powercode Integration, use

```shell
python3 integrationPowercode.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can modify the network.json file manually to reflect Site/AP bandwidth limits.
ShapedDevices.csv will be overwritten every time the Powercode integration is run.
You have the option to run integrationPowercode.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_powercode = true``` in `/etc/lqos.conf`.

## Sonar Integration

First, set the relevant parameters for Sonar (sonar_api_key, sonar_api_url, etc.) in `/etc/lqos.conf`.

To test the Sonar Integration, use

```shell
python3 integrationSonar.py
```

On the first successful run, it will create a ShapedDevices.csv file.
Built-in integrations do not overwrite an existing `network.json`; keep DIY `network.json` operator-owned.
You can modify the network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Sonar integration is run.
You have the option to run integrationSonar.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_sonar = true``` in `/etc/lqos.conf`.

## Third-Party Tools

### Jesync UI Tool Dashboard
Jesync UI Tool Dashboard is a modern, web-based control panel designed to make managing LibreQoS and its integration files easier, faster, and more user-friendly.

[https://github.com/jesienazareth/jesync_dashboard](https://github.com/jesienazareth/jesync_dashboard)

### MikroTik PPPoE Integration
This script automates the synchronization of MikroTik PPP secrets (e.g., PPPoE users) and active hotspot users with a LibreQoS-compatible CSV file (ShapedDevices.csv). It continuously monitors the MikroTik router for changes to PPP secrets and active hotspot users, such as additions, updates, or deletions, and updates the CSV file accordingly. The script also calculates rate limits (download/upload speeds) based on the assigned PPP profile and ensures the CSV file is always up-to-date.

The script is designed to run as a background service using systemd, ensuring it starts automatically on boot and restarts in case of failures.

[https://github.com/Kintoyyy/MikroTik-LibreQos-Integration](https://github.com/Kintoyyy/MikroTik-LibreQos-Integration)
