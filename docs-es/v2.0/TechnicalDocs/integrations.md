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

Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

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

# UISP's reported AP capacities for AirMax can be a bit optimistic. For new installs, we limit
# to 80% of what UISP claims an AP's capacity is, by default. This is adjustable.
airmax_capacity = 0.8

# When UISP reports only aggregate AirMax AP capacity for flexible framing and does not expose
# the live downlink ratio, use this fallback split. 0.8 means 80/20 download/upload.
airmax_flexible_frame_download_ratio = 0.8

# UISP's reported AP capacities for LTU are more accurate, and new installs now default to 100%
# of those capacities. This is adjustable.
ltu_capacity = 1.0

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
Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

ShapedDevices.csv will be overwritten every time the UISP integration is run.

Cuando varios sitios cliente de UISP comparten el mismo nombre, LibreQoS ahora intenta diferenciar los nombres visibles generados para circuitos/sitios con un sufijo más humano, como el primer segmento de la dirección, recurriendo al nombre del servicio y luego a un ID corto solo cuando hace falta. La identidad estable del circuito sigue viniendo del ID de sitio/servicio de UISP, no del nombre visible.

Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.

You have the option to run `uisp_integration` automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`. Once set, run `sudo systemctl restart lqos_scheduler`.

### UISP Overrides

Puede usar las siguientes entradas de override para reflejar su red con mayor precisión:
- `Rate Override` en la página del árbol, guardado como overrides operatorios `AdjustSiteSpeed` en `lqos_overrides.json`
- `Topology Override` en la página del árbol para nodos compatibles de UISP `full`, guardado en `lqos_overrides.json`
- integrationUISPbandwidths.csv solo como entrada heredada de compatibilidad

Las compilaciones UISP actuales auto-migran un `integrationUISPbandwidths.csv` heredado hacia overrides operatorios `AdjustSiteSpeed` en la siguiente ejecución de la integración cuando todavía no existen overrides de tasa del operador. Si ya existen, el CSV se ignora.
Las entradas JSON heredadas `uisp.bandwidth_overrides` se ignoran. La ruta autoritativa para overrides de ancho de banda es `AdjustSiteSpeed` en `lqos_overrides.json`.
Las compilaciones UISP actuales ignoran las entradas heredadas `uisp.route_overrides` en `lqos_overrides.json` y los archivos heredados `integrationUISProutes.csv`. Si existe cualquiera de los dos, LibreQoS registra una advertencia y usa la topología detectada más los overrides de Topology Manager.

Las plantillas de los archivos heredados siguen disponibles en `/opt/libreqos/src`. Si no las encuentra allí, puede obtenerlas [aquí](https://github.com/LibreQoE/LibreQoS/tree/develop/src). Para cambios nuevos de ancho de banda, prefiera los overrides operatorios en `lqos_overrides.json`.
Para la intención de camino, use la selección de padre y la preferencia de attachments en Topology Manager. Ese es ahora el reemplazo soportado para los antiguos overrides de costo de ruta de UISP.

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
Built-in integrations do not overwrite `network.json`; keep DIY `network.json` operator-owned.
You can modify the network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Sonar integration is run.
You have the option to run integrationSonar.py automatically on boot and every X minutes (set by the parameter `queue_refresh_interval_mins`), which is highly recommended. This can be enabled by setting ```enable_sonar = true``` in `/etc/lqos.conf`.
