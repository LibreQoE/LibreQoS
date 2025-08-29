# Integraciones CRM/NMS

  * [Integración con Splynx](#integración-con-splynx)
    + [Acceso API de Splynx](#acceso-api-de-splynx)
    + [Sobrescrituras de Splynx](#sobrescrituras-de-splynx)
  * [Integración con UISP](#integración-con-uisp)
    + [Sobrescrituras de UISP](#sobrescrituras-de-uisp)
      - [Sobrescrituras de Rutas UISP](#sobrescrituras-de-rutas-uisp)
  * [Integración con WISPGate](#integración-con-wispgate)
  * [Integración con Powercode](#integración-con-powercode)
  * [Integración con Sonar](#integración-con-sonar)

## Integración con Splynx

> **Nota para usuarios existentes:** Si está actualizando desde una versión anterior a v1.5-RC-2 y experimenta cambios inesperados en la topología, agregue `strategy = "full"` para mantener el comportamiento anterior. El nuevo valor predeterminado `ap_only` proporciona mejor distribución de CPU para redes grandes.

Primero, configure los parámetros relevantes de Splynx en `/etc/lqos.conf`:

```
[spylnx_integration]
enable_spylnx = true
strategy = "ap_only"  # Estrategia de topología (flat, ap_only, ap_site, full)
api_key = "su_api_key"
api_secret = "su_api_secret"
url = "https://su-instancia-splynx.com"
```

### Acceso API de Splynx

La integración con Splynx utiliza autenticación Básica. Para usar este tipo de autenticación, asegúrese de habilitar el [Acceso No Seguro](https://splynx.docs.apiary.io/#introduction/authentication) en la configuración de su clave API de Splynx. Además, la clave API de Splynx debe tener los permisos adecuados.

| Categoría       | Nombre                         | Permiso |
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

Para probar la integración con Splynx, use:

```shell
python3 integrationSplynx.py
```

En la primera ejecución exitosa, se crearán los archivos ShapedDevices.csv y network.json.
ShapedDevices.csv será sobrescrito cada vez que se ejecute la integración con Splynx.

Para asegurarse de que network.json siempre se sobrescriba con la versión más reciente obtenida por la integración, edite `/etc/lqos.conf` con el comando `sudo nano /etc/lqos.conf` y configure el valor `always_overwrite_network_json` a `true`.
Luego ejecute `sudo systemctl restart lqosd`.

Tiene la opción de ejecutar integrationSplynx.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_spylnx = true``` en `/etc/lqos.conf`.
Una vez configurado, ejecute `sudo systemctl restart lqos_scheduler`.

### Estrategias de Topología de Splynx

LibreQoS admite múltiples estrategias de topología para optimizar la distribución de carga de CPU en redes grandes. Configure la estrategia en `/etc/lqos.conf` bajo `[spylnx_integration]`:

```
[spylnx_integration]
enable_spylnx = true
strategy = "ap_only"  # Estrategia predeterminada
```

Estrategias disponibles:

| Estrategia | Descripción | Caso de Uso |
|-----------|-------------|-------------|
| `flat` | Solo regula suscriptores, ignorando todas las relaciones de nodos padre | Distribución máxima de CPU, jerarquía mínima |
| `ap_only` | Capa única de AP + Clientes (predeterminado) | Redes grandes que necesitan amplia distribución de CPU |
| `ap_site` | Estructura Sitio → AP → Clientes | Redes grandes con mejor organización |
| `full` | Topología completa: Sitios → Enlaces → APs → Clientes | Topología detallada, redes más pequeñas |

**⚠️ Aviso de Cambio Importante:** Antes de v1.5-RC-2, el comportamiento predeterminado era equivalente a `full`. Si está actualizando y desea mantener el comportamiento anterior, agregue `strategy = "full"` a su configuración.

#### Eligiendo la Estrategia Correcta

- **`flat`**: Mejor para redes que experimentan severa congestión de CPU. Distribuye la carga al máximo pero pierde toda la información de jerarquía.
- **`ap_only`** (predeterminado): Recomendado para la mayoría de redes grandes. Proporciona buena distribución de CPU manteniendo las asociaciones de AP.
- **`ap_site`**: Buen equilibrio entre organización y rendimiento. Los sitios permanecen como padres de nivel superior.
- **`full`**: Use para redes más pequeñas o cuando se requiera visualización completa de topología.

### Sobrescrituras de Splynx

También puede modificar el archivo `integrationSplynxBandwidths.csv` para sobrescribir los anchos de banda predeterminados de cada Nodo (Sitio, AP).

Hay una plantilla disponible en la carpeta `/opt/libreqos/src`. Para usarla, copie el archivo `integrationSplynxBandwidths.template.csv` (eliminando la parte `.template` del nombre) y edítelo con la información correspondiente. Por ejemplo, si desea cambiar el bando de ancha de un sitio, ejecutaría:
```
sudo cp /opt/libreqos/src/integrationSplynxBandwidths.template.csv /opt/libreqos/src/integrationSplynxBandwidths.csv
```
Y luego editaría el CSV con LibreOffice o el editor de CSV de su preferencia.

### Consideraciones de Rendimiento

Para redes con miles de suscriptores que experimentan congestión de CPU:

1. **Monitorear uso de núcleos de CPU** - Verifique si los nodos de nivel superior están causando cuellos de botella de CPU usando `htop` o `lqtop`
2. **Cambiar estrategias** - Cambie de `full` a `ap_only` o `flat` para mejor distribución
3. **Probar incrementalmente** - Pruebe `ap_site` primero, luego `ap_only` si es necesario
4. **Verificar regulación** - Asegúrese de que los límites de ancho de banda se apliquen correctamente después de cambios de estrategia

Para cambiar estrategias:
```bash
sudo nano /etc/lqos.conf
# Edite strategy = "estrategia_deseada" bajo [spylnx_integration]
sudo systemctl restart lqos_scheduler
```

Monitorear el impacto del cambio:
```bash
# Verificar distribución de CPU
lqtop

# Verificar que la regulación de clientes funciona
python3 /opt/libreqos/src/integrationSplynx.py
```

## Integración con UISP

Primero, configure los parámetros relevantes de UISP (token, url, automatic_import_uisp, etc.) en `/etc/lqos.conf`.
```
# Ejecutar integración con UISP automáticamente en el servicio lqos_scheduler
enable_uisp = true

# Token de acceso API de UISP
token = ""

# URL de su UISP (incluir https://, pero omitir lo que sigue después de .com, .net, etc.)
url = "https://uisp.your_domain.com"

# El sitio aquí se refiere al "Root site" desde el cual UISP generará su topología de red.
# El valor predeterminado es un "string" en blanco.
site = "Site_name"

# Tipo de Estrategia. "full" es recomendada. "flat" puede ser utilizada si solo desea regular clientes.
strategy = "full"

# Estrategia de suspensión:
# * "none" - no manejar suspensiones
# * "ignore" - no agregar clientes suspendidos al mapa de red
# * "slow" - limitar clientes suspendidos a 1 Mbps
suspended_strategy = "none"

# Las capacidades de los AP reportadas por UISP para AirMax pueden ser un poco optimistas. Para los AP AirMax, limitamos al 65% de lo que UISP afirma que es la capacidad de un AP, de forma predeterminada. Esto es ajustable.
airmax_capacity = 0.65

# Las capacidades de los AP reportadas por UISP para LTU son más precisas, pero para mayor seguridad las ajustamos al 95% de dichas capacidades. Esto es ajustable.
ltu_capacity = 0.95

# Si desea excluir sitios en UISP para que no aparezcan en su network.json de LibreQoS, simplemente
# inclúyalos aquí. Por ejemplo, exclude_sites = ["Site_1", "Site_2"]
exclude_sites = []

# Si usa DHCPv6 y desea importar los CIDR de IPv6 correspondientes a cada dirección IPv4
# de los clientes, puede hacerlo con esta opción. Si está habilitada, asegúrese de completar
# el archivo mikrotikDHCPRouterList.csv y ejecutar `python3 mikrotikFindIPv6.py` para probar su funcionalidad.
ipv6_with_mikrotik = false

# Si desea que los clientes reciban un poco más o menos de su plan de velocidad asignado, configúrelo aquí.
# Por ejemplo, 1.15 equivale a un 15% por encima de su plan asignado.
bandwidth_overhead_factor = 1.15

# De forma predeterminada, el "mínimo" de los clientes se establece al 98% del máximo (CIR).
commit_bandwidth_multiplier = 0.98
exception_cpes = []

# Si tiene algunos sitios conectados a través de APs PtMP, configure `true`
use_ptmp_as_parent = true
uisp_use_burst = true
```

Para probar la integración con UISP, ejecute:

```shell
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```
En la primera ejecución exitosa, se crearán los archivos network.json y ShapedDevices.csv.
Si ya existe un archivo network.json, no será sobrescrito a menos que configure ```always_overwrite_network_json = true```.

ShapedDevices.csv será sobrescrito cada vez que se ejecute la integración de UISP.

Para asegurarse de que network.json siempre sea sobrescrito con la versión más reciente obtenida por la integración, edite el archivo `/etc/lqos.conf` con el comando `sudo nano /etc/lqos.conf` y configure el valor  `always_overwrite_network_json` a `true`.
Luego ejecute: `sudo systemctl restart lqosd`.

Tiene la opción de ejecutar integrationUISP.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_uisp = true``` en `/etc/lqos.conf`. Una vez configurado, ejecute `sudo systemctl restart lqos_scheduler`.

### Sobrescrituras de UISP

También puede modificar los siguientes archivos para reflejar su red con mayor precisión:
- integrationUISPbandwidths.csv
- integrationUISProutes.csv

Cada uno de los archivos mencionados arriba tienen plantillas, las cuales están disponibles en la carpeta `/opt/libreqos/src`. Si no los encuentra, puede obtenerlos [aquí](https://github.com/LibreQoE/LibreQoS/tree/develop/src). Para utilizarlos, copie el archivo (eliminando la parte `.template` del nombre) y edítelo con la información correspondiente.
Por ejemplo, si desea cambiar el bando de ancha de un sitio, ejecutaría:
```
sudo cp /opt/libreqos/src/integrationUISPbandwidths.template.csv /opt/libreqos/src/integrationUISPbandwidths.csv
```
Y luego editaría el CSV con LibreOffice o el editor de CSV de su preferencia.

#### Sobrescrituras de Rutas UISP

El costo predeterminado entre nodos suele ser 10. La integración genera un archivo de gráfico en formato dot: `/opt/libreqos/src/graph.dot` el cuál puede visualizarse con [Graphviz](https://dreampuf.github.io/GraphvizOnline/). Esto genera un mapa con los costos de todos los enlaces.

![image](https://github.com/user-attachments/assets/4edba4b5-c377-4659-8798-dfc40d50c859)

Ejemplo:
Tiene Sitio 1, Sitio 2 y Sitio 3.
Existe un camino de respaldo entre Sitio 1 y Sitio 3, pero no es el preferido.
El camino preferido debería ser: Sitio 1 > Sitio 2 > Sitio 3, pero la integración conecta directamente Sitio 1 > Sitio 3 por predeterminado.

Para solucionar esto, agregue un costo mayor al predeterminado entre Sitio 1 y Sitio 3:
```
Site 1, Site 3, 100
Site 3, Site 1, 100
```
De esta forma, el tráfico seguirá el camino: Sitio 1 > Sitio 2 > Sitio 3.

Para aplicar el cambio, reinicie la integración ejecutando: ```sudo systemctl restart lqos_scheduler```.

## Integración con WISPGate
Primero, configure los parámetros relevantes de WISPGate en `/etc/lqos.conf`.
Debería haber una sección como la siguiente:

```
[wispgate_integration]
enable_wispgate = false
wispgate_api_token = "token"
wispgate_api_url = "https://your_wispgate_url.com"
```

Si la sección no existe, agréguela copiando el bloque anterior.
Después, configure los valores apropiados para wispgate_api_token y wispgate_api_url, luego guarde el archivo.

Para probar la integración con WISPGate, ejecute:

```shell
python3 integrationWISPGate.py
```

En la primera ejecución exitosa, se crearán los archivos network.json y ShapedDevices.csv.
ShapedDevices.csv será sobrescrito cada vez que se ejecute la integración de WISPGate.

Para asegurarse de que network.json siempre sea sobrescrito con la versión más reciente obtenida por la integración, edite el archivo `/etc/lqos.conf` con el comando `sudo nano /etc/lqos.conf` y configure el valor  `always_overwrite_network_json` a `true`.
Luego ejecute: `sudo systemctl restart lqosd`.

Tiene la opción de ejecutar integrationWISPGate.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_wispgate = true``` en `/etc/lqos.conf`. Una vez configurado, ejecute `sudo systemctl restart lqos_scheduler`.

## Integración con Powercode

Primero, configure los parámetros relevantes de Powercode (powercode_api_key, powercode_api_url, etc.) en `/etc/lqos.conf`.

Para probar la integración con Powercode, ejecute:

```shell
python3 integrationPowercode.py
```

En la primera ejecución exitosa, se creará el archivo ShapedDevices.csv.
Puede modificar el archivo network.json manualmente para reflejar los límites de ancho de banda de los Sitios/AP.
El archivo ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración de Powercode.
Tiene la opción de ejecutar integrationPowercode.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_powercode = true``` en `/etc/lqos.conf`.

## Integración con Sonar

Primero, configure los parámetros relevantes de Sonar (sonar_api_key, sonar_api_url, etc.) en `/etc/lqos.conf`.

Para probar la integración con Sonar, ejecute:

```shell
python3 integrationSonar.py
```

En la primera ejecución exitosa, se crearán los archivos network.json y ShapedDevices.csv.
Si ya existe un archivo network.json, no será sobrescrito a menos que configure ```always_overwrite_network_json = true```.
Puede modificar el archivo network.json para reflejar con mayor precisión los límites de ancho de banda.
El archivo ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración de Sonar.
Tiene la opción de ejecutar integrationSonar.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_sonar = true``` en `/etc/lqos.conf`.

## Herramientas de Terceros

### Jesync UI Tool Dashboard
Jesync UI Tool Dashboard es un panel de control moderno, basado en la web, diseñado para facilitar, agilizar y hacer más amigable la gestión de LibreQoS y de sus archivos de integración.

[https://github.com/jesienazareth/jesync_dashboard](https://github.com/jesienazareth/jesync_dashboard)

### Integración MikroTik PPPoE
Este script automatiza la sincronización de los secretos PPP de MikroTik (por ejemplo, usuarios PPPoE) y los usuarios activos de hotspot con un archivo CSV compatible con LibreQoS (ShapedDevices.csv). Supervisa continuamente el router MikroTik para detectar cambios en los secretos PPP y en los usuarios activos de hotspot, como adiciones, actualizaciones o eliminaciones, y actualiza el archivo CSV en consecuencia.
El script también calcula los límites de velocidad (descarga/carga) según el perfil PPP asignado y garantiza que el archivo CSV siempre esté actualizado.

El script está diseñado para ejecutarse como un servicio en segundo plano utilizando systemd, asegurando que se inicie automáticamente al arrancar el sistema y se reinicie en caso de algún fallo.

[https://github.com/Kintoyyy/MikroTik-LibreQos-Integration](https://github.com/Kintoyyy/MikroTik-LibreQos-Integration)
