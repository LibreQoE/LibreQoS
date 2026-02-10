# Integrations

## Splynx Integration

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

### Splynx API Access

The Splynx Integration uses Basic authentication. For using this type of authentication, please make sure you enable [Unsecure access](https://splynx.docs.apiary.io/#introduction/authentication) in your Splynx API key settings. Also the Splynx API key should be granted access to the necessary permissions.

* Tariff Plans -> Internet -> view
* Tariff Plans -> Bundle -> view
* Tariff Plans -> One time -> view
* Tariff Plans -> Recurring  -> view
* FUP -> Counter -> view
* FUP -> Compiler -> view
* FUP -> Policies -> view
* FUP -> Capped Data -> view
* FUP -> CAP Tariff -> view
* FUP -> FUP Limits -> view
* FUP -> Traffic Usage -> view
* Customers -> customer -> view
* Customers -> customer information -> view
* Customers -> Customers online -> view
* Customers -> customer bundle services -> view
* Customers -> customer internet services -> view
* Customers -> traffic counter -> view
* Customers -> customer recurring services -> view
* Customers -> bonus traffic counter -> view
* Customers -> CAP history -> view
* Networking -> routers -> view
* Networking -> network sites >view
* Networking -> router contention -> view
* Networking -> IPv4 networks -> view
* Networking -> IPv4 networks IP -> view

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
# * "slow" - limit suspended customers to 0.1 Mbps
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

The default cost between nodes is 10.

Say you have Site 1, Site 2, and Site 3.
A backup path exists between Site 1 and Site 3, but is not the preferred path.
Your preference is Site 1 > Site 2 > Site 3, but the integration by default connects Site 1 > Site 3 directly.

To fix this, add a cost above the default for the path between Site 1 and Site 3.
```
Site 1, Site 3, 100
```
With this, data will flow Site 1 > Site 2 > Site 3.

To make the change, perform a reload of the integration with ```sudo systemctl restart lqos_scheduler```.

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
