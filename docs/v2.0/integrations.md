# CRM/NMS Integrations

  * [Splynx Integration](#splynx-integration)
    + [Splynx API Access](#splynx-api-access)
    + [Splynx Overrides](#splynx-overrides)
  * [UISP Integration](#uisp-integration)
    + [UISP Overrides](#uisp-overrides)
      - [UISP Route Overrides](#uisp-route-overrides)
  * [WISPGate Integration](#wispgate-integration)
  * [Powercode Integration](#powercode-integration)
  * [Sonar Integration](#sonar-integration)

## Splynx Integration

> **Note for existing users:** If you're upgrading from a version before v1.5-RC-2 and experience unexpected topology changes, add `strategy = "full"` to maintain previous behavior. The new default `ap_only` provides better CPU distribution for large networks.

First, set the relevant parameters for Splynx in `/etc/lqos.conf`:

```
[spylnx_integration]
enable_spylnx = true
strategy = "ap_only"  # Topology strategy (flat, ap_only, ap_site, full)
api_key = "your_api_key"
api_secret = "your_api_secret"
url = "https://your-splynx-instance.com"
```

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

You have the option to run integrationSplynx.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_spylnx = true``` in `/etc/lqos.conf`.
Once set, run `sudo systemctl restart lqos_scheduler`.

### Splynx Topology Strategies

LibreQoS supports multiple topology strategies to optimize CPU load distribution for large networks. Configure the strategy in `/etc/lqos.conf` under `[spylnx_integration]`:

```
[spylnx_integration]
enable_spylnx = true
strategy = "ap_only"  # Default strategy
```

Available strategies:

| Strategy | Description | Use Case |
|----------|-------------|----------|
| `flat` | Only shapes subscribers, ignoring all parent node relationships | Maximum CPU distribution, minimal hierarchy |
| `ap_only` | Single layer of AP + Clients (default) | Large networks needing wide CPU spread |
| `ap_site` | Site → AP → Clients structure | Large networks with better organization |
| `full` | Complete topology: Sites → Backhauls → APs → Clients | Detailed topology, smaller networks |

**⚠️ Breaking Change Notice:** Prior to v1.5-RC-2, the default behavior was equivalent to `full`. If upgrading and you want to maintain the previous behavior, add `strategy = "full"` to your configuration.

#### Choosing the Right Strategy

- **`flat`**: Best for networks experiencing severe CPU congestion. Spreads load maximally but loses all hierarchy information.
- **`ap_only`** (default): Recommended for most large networks. Provides good CPU distribution while maintaining AP associations.
- **`ap_site`**: Good balance between organization and performance. Sites remain top-level parents.
- **`full`**: Use for smaller networks or when complete topology visualization is required.

### Splynx Overrides

You can also modify the the file `integrationSplynxBandwidths.csv` to override the default bandwidths for each Node (Site, AP).

A template is available in the `/opt/libreqos/src` folder. To utilize the template, copy the file `integrationSplynxBandwidths.template.csv` (removing the `.template` part of the filename) and set the appropriate information inside each file. For example, if you want to change the set bandwidth for a site, you would do:
```
sudo cp /opt/libreqos/src/integrationSplynxBandwidths.template.csv /opt/libreqos/src/integrationSplynxBandwidths.csv
```
And edit the CSV using LibreOffice or your preferred CSV editor.

### Performance Considerations

For networks with thousands of subscribers experiencing CPU congestion:

1. **Monitor CPU core usage** - Check if top-level nodes are causing CPU bottlenecks using `htop` or `lqtop`
2. **Switch strategies** - Move from `full` to `ap_only` or `flat` for better distribution
3. **Test incrementally** - Try `ap_site` first, then `ap_only` if needed
4. **Verify shaping** - Ensure bandwidth limits are still applied correctly after strategy changes

To change strategies:
```bash
sudo nano /etc/lqos.conf
# Edit strategy = "desired_strategy" under [spylnx_integration]
sudo systemctl restart lqos_scheduler
```

Monitor the change impact:
```bash
# Check CPU distribution
lqtop

# Verify customer shaping is working
python3 /opt/libreqos/src/integrationSplynx.py
```

## UISP Integration

First, set the relevant parameters for UISP (token, url, automatic_import_uisp, etc.) in `/etc/lqos.conf`.
```
# Whether to run the UISP integration automatically in the lqos_scheduler service
enable_uisp = true

# Your UISP API Access Token
token = ""

# Your UISP URL (include https://, but omit anything past .com, .net, etc)
url = "https://uisp.your_domain.com"

# The site here refers to the Root site you want UISP to base its topology "perspective" from.
# Default value is a blank string.
site = "Site_name"

# Strategy type. "full" is recommended. "flat" can be used if only client shaping is desired.
strategy = "full"

# Suspension strategy:
# * "none" - do not handle suspensions
# * "ignore" - do not add suspended customers to the network map
# * "slow" - limit suspended customers to 1mbps
suspended_strategy = "none"

# UISP's reported AP capacities for AirMax can be a bit optimistic. For AirMax APs, we limit
# to 65% of what UISP claims an AP's capacity is, by default. This is adjustable.
airmax_capacity = 0.65

# UISP's reported AP capacities for LTU are more accurate, but to be safe we adjust to 95%
# of those capacities. This is adjustable.
ltu_capacity = 0.95

# If you want to exclude sites in UISP from appearing in your LibreQoS network.json, simply
# include them here. For example, exclude_sites = ["Site_1", "Site_2"]
exclude_sites = []

# If you use DHCPv6, and want to pull in IPv6 CIDRs corresponding to each customer's IPv4
# address, you can do so with this. If enabled, be sure to fill out mikrotikDHCPRouterList.csv
# and run `python3 mikrotikFindIPv6.py` to test its functionality.
ipv6_with_mikrotik = false

# If you want customers to recieve a bit more of less than their allocated speed plan, set
# it here. For example, 1.15 is 15% above their alloted speed plan.
bandwidth_overhead_factor = 1.15

# By default, the customer "minimum" is set to 98% of the maximuum (CIR).
commit_bandwidth_multiplier = 0.98
exception_cpes = []

# If you have some sites branched off PtMP Access Points, set `true`
use_ptmp_as_parent = true
uisp_use_burst = true
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
