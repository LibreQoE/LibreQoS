# CRM/NMS Integrations

  * [Splynx Integration](#splynx-integration)
    + [Topology Strategies](#topology-strategies)
    + [Promote to Root Nodes (Performance Optimization)](#promote-to-root-nodes-performance-optimization)
    + [Splynx API Access](#splynx-api-access)
    + [Splynx Overrides](#splynx-overrides)
  * [Netzur Integration](#netzur-integration)
  * [UISP Integration](#uisp-integration)
    + [Topology Strategies](#topology-strategies-1)
    + [Suspension Handling Strategies](#suspension-handling-strategies)
    + [Burst](#burst)
    + [Configuration Example](#configuration-example)
    + [UISP Overrides](#uisp-overrides)
      - [UISP Route Overrides](#uisp-route-overrides)
  * [WISPGate Integration](#wispgate-integration)
  * [Powercode Integration](#powercode-integration)
  * [Sonar Integration](#sonar-integration)

## What these integrations do

These integrations work by synchronizing with your CRM/NMS system to produce the two files LibreQoS requires for shaping - [network.json](configuration.md#network-hierarchy) and [ShapedDevices.csv](configuration.md#circuits).

## Splynx Integration

> **⚠️ Breaking Change Notice**: Prior to v1.5-RC-2, the default Splynx strategy was `full`. Starting with v1.5-RC-2, the default strategy is `ap_only` for improved CPU performance. If you require the previous behavior, explicitly set `strategy = "full"` in your Splynx configuration section.

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

### Topology Strategies

LibreQoS supports multiple topology strategies for Splynx integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers, no hierarchy | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Single layer: AP → Clients | Low | **Default**. Best balance of performance and structure |
| `ap_site` | Two layers: Site → AP → Clients | Medium | Site-level aggregation with moderate complexity |
| `full` | Complete topology mapping | Highest | Full network hierarchy representation |

Configure the strategy in `/etc/lqos.conf` under the `[splynx]` section:

```ini
[splynx]
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

On the first successful run, it will create a ShapedDevices.csv file and network.json.
ShapedDevices.csv will be overwritten every time the Splynx integration is run.

To ensure the network.json is always overwritten with the newest version pulled in by the integration, please edit `/etc/lqos.conf` with the command `sudo nano /etc/lqos.conf`.
Edit the file to set the value of `always_overwrite_network_json` to `true`.
Then, run `sudo systemctl restart lqosd`.

You have the option to run integrationSplynx.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_splynx = true``` under the ```[splynx_integration]``` section in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.

### Splynx Overrides

You can also modify the the file `integrationSplynxBandwidths.csv` to override the default bandwidths for each Node (Site, AP).

A template is available in the `/opt/libreqos/src` folder. To utilize the template, copy the file `integrationSplynxBandwidths.template.csv` (removing the `.template` part of the filename) and set the appropriate information inside each file. For example, if you want to change the set bandwidth for a site, you would do:
```
sudo cp /opt/libreqos/src/integrationSplynxBandwidths.template.csv /opt/libreqos/src/integrationSplynxBandwidths.csv
```
And edit the CSV using LibreOffice or your preferred CSV editor.

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
- `use_mikrotik_ipv6` enriches subscriber devices with IPv6 prefixes discovered via `mikrotikDHCPRouterList.csv`.

Run a manual import with:

```bash
python3 integrationNetzur.py
```

The integration regenerates `ShapedDevices.csv` and, unless `always_overwrite_network_json` is disabled, updates `network.json`. Adjust the Integration → Common settings if you need to preserve an existing network map between Netzur syncs.

## UISP Integration

First, set the relevant parameters for UISP in `/etc/lqos.conf`.

### Topology Strategies

LibreQoS supports multiple topology strategies for UISP integration to balance CPU performance with network hierarchy needs:

| Strategy | Description | CPU Impact | Use Case |
|----------|-------------|------------|----------|
| `flat` | Only shapes subscribers by service plan speed | Lowest | Maximum performance, simple subscriber-only shaping |
| `ap_only` | Shapes by service plan and Access Point | Low | Good balance of performance and AP-level control |
| `ap_site` | Shapes by service plan, Access Point, and Site | Medium | Site-level aggregation with moderate complexity |
| `full` | Shapes entire network including backhauls, Sites, APs, and clients | Highest | **Recommended for most deployments**. Complete network hierarchy with backhaul awareness |

**Choosing the Right Strategy:**
- Use `full` for most deployments to get complete network topology awareness including backhaul links
- Use `ap_site` if you need site-level control but don't need backhaul shaping
- Use `ap_only` for better performance when site aggregation isn't needed
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
strategy = "full"  # Recommended for most deployments

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
always_overwrite_network_json = false  # Set true to rebuild topology each run
exception_cpes = []  # CPE exceptions in ["cpe:parent"] format
```

To test the UISP Integration, use

```shell
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```

On the first successful run, it will create a network.json and ShapedDevices.csv file.
If a network.json file exists, it will not be overwritten, unless you set ```always_overwrite_network_json = true```.

ShapedDevices.csv will be overwritten every time the UISP integration is run.

To ensure the network.json is always overwritten with the newest version pulled in by the integration, please edit `/etc/lqos.conf` with the command `sudo nano /etc/lqos.conf`.
Edit the file to set the value of `always_overwrite_network_json` to `true`.
Then, run `sudo systemctl restart lqosd`.

You have the option to run integrationUISP.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`. Once set, run `sudo systemctl restart lqos_scheduler`.

### UISP Overrides

You can also modify the the following files to more accurately reflect your network:
- integrationUISPbandwidths.csv
- integrationUISProutes.csv

Each of the files above have templates available in the `/opt/libreqos/src` folder. If you don't find them there, you can navigate [here](https://github.com/LibreQoE/LibreQoS/tree/develop/src). To utilize the template, copy the file (removing the `.template` part of the filename) and set the appropriate information inside each file.
For example, if you want to change the set bandwidth for a site, you would do:
```
sudo cp /opt/libreqos/src/integrationUISPbandwidths.template.csv /opt/libreqos/src/integrationUISPbandwidths.csv
```
And edit the CSV using LibreOffice or your preferred CSV editor.

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

To ensure the network.json is always overwritten with the newest version pulled in by the integration, please edit `/etc/lqos.conf` with the command `sudo nano /etc/lqos.conf`.
Edit the file to set the value of `always_overwrite_network_json` to `true`.
Then, run `sudo systemctl restart lqosd`.

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
If a network.json file exists, it will not be overwritten, unless you set ```always_overwrite_network_json = true```.
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
