# Integrations

## Splynx Integration

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

The Splynx Integration uses Basic authentication. For using this type of authentication, please make sure you enable [Unsecure access](https://splynx.docs.apiary.io/#introduction/authentication) in your Splynx API key settings. Also the Splynx API key should be granted access to the necessary permissions.

To test the Splynx Integration, use

```shell
python3 integrationSplynx.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can manually create your network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Splynx integration is run.
You have the option to run integrationSplynx.py automatically on boot and every 5 minutes, which is recommended. This can be enabled by setting ```enable_spylnx = true``` in `/etc/lqos.conf`.

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
cd /opt/libreqos/src/rust/uisp_integration
sudo cargo run
```

On the first successful run, it will create a network.json and ShapedDevices.csv file.
If a network.json file exists, it will not be overwritten, unless you set ```always_overwrite_network_json = true```.
You can modify the the following files to more accurately reflect your network:
- integrationUISPbandwidths.csv
- integrationUISProutes.csv

ShapedDevices.csv will be overwritten every time the UISP integration is run.
You have the option to run integrationUISP.py automatically on boot and every 5 minutes, which is recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`.

## Powercode Integration

First, set the relevant parameters for Powercode (powercode_api_key, powercode_api_url, etc.) in `/etc/lqos.conf`.

To test the Powercode Integration, use

```shell
python3 integrationPowercode.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can modify the network.json file manually to reflect Site/AP bandwidth limits.
ShapedDevices.csv will be overwritten every time the Powercode integration is run.
You have the option to run integrationPowercode.py automatically on boot and every 5 minutes, which is recommended. This can be enabled by setting ```enable_powercode = true``` in `/etc/lqos.conf`.

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
You have the option to run integrationSonar.py automatically on boot and every 5 minutes, which is recommended. This can be enabled by setting ```enable_sonar = true``` in `/etc/lqos.conf`.
