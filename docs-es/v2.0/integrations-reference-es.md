# Integraciones CRM/NMS

Esta página conserva material heredado de referencia detallada.
La guía canónica y orientada a tareas está en las páginas por integración enlazadas desde [Integraciones CRM/NMS](integrations-es.md).

  * [Integración con Splynx](#integración-con-splynx)
    + [Estrategias de Topología](#estrategias-de-topología)
    + [Promover Nodos a Raíz (Optimización de Rendimiento)](#promover-nodos-a-raíz-optimización-de-rendimiento)
    + [Acceso API de Splynx](#acceso-api-de-splynx)
    + [Sobrescrituras de Splynx](#sobrescrituras-de-splynx)
  * [Integración con Netzur](#integración-con-netzur)
  * [Integración con VISP](#integración-con-visp)
  * [Integración con UISP](#integración-con-uisp)
    + [Estrategias de Topología](#estrategias-de-topología-1)
    + [Estrategias de Manejo de Suspensiones](#estrategias-de-manejo-de-suspensiones)
    + [Burst](#burst)
    + [Ejemplo de Configuración](#ejemplo-de-configuración)
    + [Sobrescrituras de UISP](#sobrescrituras-de-uisp)
      - [Sobrescrituras de Rutas UISP](#sobrescrituras-de-rutas-uisp)
  * [Integración con WISPGate](#integración-con-wispgate)
  * [Integración con Powercode](#integración-con-powercode)
  * [Integración con Sonar](#integración-con-sonar)

La mayoría de operadores usan este camino de integraciones incluidas.
Si usa scripts propios como fuente de verdad para `network.json` y `ShapedDevices.csv`, comience por [Modos de operación y fuente de verdad](operating-modes-es.md).

Comportamiento importante cuando hay integraciones habilitadas:
- `ShapedDevices.csv` normalmente se regenera por los jobs de sincronización.
- El comportamiento de sobrescritura de `network.json` depende de su configuración de integración (por ejemplo `always_overwrite_network_json`).
- Las ediciones manuales pueden sobrescribirse en el próximo ciclo de refresco.

## Integración con Splynx

> **⚠️ Aviso de Cambio Importante**: Antes de v1.5-RC-2, la estrategia predeterminada de Splynx era `full`. A partir de v1.5-RC-2, la estrategia predeterminada es `ap_only` para un mejor rendimiento del CPU. Si requiere el comportamiento anterior, configure explícitamente `strategy = "full"` en su sección de configuración de Splynx.

Primero, configure los parámetros relevantes de Splynx (splynx_api_key, splynx_api_secret, etc.) en `/etc/lqos.conf`.

### Estrategias de Topología

LibreQoS soporta múltiples estrategias de topología para la integración con Splynx, balanceando el rendimiento del CPU con las necesidades de jerarquía de red:

| Estrategia | Descripción | Impacto CPU | Caso de Uso |
|------------|-------------|-------------|-------------|
| `flat` | Solo regula suscriptores, sin jerarquía | Menor | Máximo rendimiento, regulación simple de suscriptores únicamente |
| `ap_only` | Una capa: AP → Clientes | Bajo | **Predeterminado**. Mejor balance entre rendimiento y estructura |
| `ap_site` | Dos capas: Sitio → AP → Clientes | Medio | Agregación a nivel de sitio con complejidad moderada |
| `full` | Mapeo completo de topología | Mayor | Representación completa de jerarquía de red |

Configure la estrategia en `/etc/lqos.conf` bajo la sección `[splynx]`:

```ini
[splynx]
# ... otras configuraciones de splynx ...
strategy = "ap_only"
```

**Consideraciones de Rendimiento:**
- Las estrategias `flat` y `ap_only` reducen significativamente la carga del CPU al limitar la profundidad de la red
- Elija `ap_only` para la mayoría de implementaciones a menos que necesite agregación de tráfico a nivel de sitio
- Use `full` solamente si requiere representación completa de la topología de red y tiene recursos CPU adecuados

### Promover Nodos a Raíz (Optimización de Rendimiento)

Cuando use la estrategia de topología `full`, puede encontrar cuellos de botella de rendimiento del CPU donde todo el tráfico fluye a través de un solo sitio raíz, limitando el throughput a lo que un solo núcleo de CPU puede manejar.

La función **promote_to_root** soluciona esto promoviendo sitios específicos a nodos de nivel raíz, distribuyendo la regulación de tráfico entre múltiples núcleos de CPU.

**Configuración:**
1. Navegue a Integración → Común en la interfaz web
2. En el campo "Promover Nodos a Raíz", ingrese un nombre de sitio por línea:
```
Sitio_Remoto_Alpha
Sitio_Remoto_Beta
Centro_Datos_Oeste
```

**Beneficios:**
- Elimina el cuello de botella de CPU único para redes con sitios remotos
- Distribuye la regulación de tráfico entre múltiples núcleos de CPU
- Mejora el rendimiento general de la red para topologías grandes
- Funciona tanto con integraciones de Splynx como de UISP

**Cuándo Usar:**
- Redes con múltiples sitios remotos de alta capacidad
- Cuando use la estrategia de topología `full` y experimente limitaciones de CPU
- Redes grandes donde el tráfico del sitio raíz excede la capacidad de un solo núcleo

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

Tiene la opción de ejecutar integrationSplynx.py automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_splynx = true``` bajo la sección ```[splynx_integration]``` en `/etc/lqos.conf`.
Una vez configurado, ejecute `sudo systemctl restart lqos_scheduler`.

### Sobrescrituras de Splynx

También puede modificar el archivo `integrationSplynxBandwidths.csv` para sobrescribir los anchos de banda predeterminados de cada Nodo (Sitio, AP).

Hay una plantilla disponible en la carpeta `/opt/libreqos/src`. Para usarla, copie el archivo `integrationSplynxBandwidths.template.csv` (eliminando la parte `.template` del nombre) y edítelo con la información correspondiente. Por ejemplo, si desea cambiar el bando de ancha de un sitio, ejecutaría:
```
sudo cp /opt/libreqos/src/integrationSplynxBandwidths.template.csv /opt/libreqos/src/integrationSplynxBandwidths.csv
```
Y luego editaría el CSV con LibreOffice o el editor de CSV de su preferencia.

## Integración con Netzur

Los despliegues de Netzur proporcionan un endpoint REST con información de zonas y clientes protegido con token Bearer. Configure la sección `[netzur_integration]` en `/etc/lqos.conf`:

```ini
[netzur_integration]
enable_netzur = true
api_key = "su-token-netzur"
api_url = "https://netzur.ejemplo.com/api/libreqos"
timeout_secs = 60
use_mikrotik_ipv6 = false
```

- `enable_netzur` habilita la importación automática desde `lqos_scheduler`.
- `api_key` es el token Bearer generado dentro de Netzur.
- `api_url` debe devolver un JSON con los arreglos `zones` (convertidos en sitios) y `customers` (convertidos en circuitos y dispositivos).
- `timeout_secs` permite incrementar el tiempo de espera de la petición cuando el API responde lentamente (por defecto 60 segundos).
- `use_mikrotik_ipv6` agrega prefijos IPv6 obtenidos de `mikrotikDHCPRouterList.csv`.

Para una importación manual:

```bash
python3 integrationNetzur.py
```

La integración actualiza `ShapedDevices.csv` y, salvo que `always_overwrite_network_json` esté deshabilitado, también `network.json`. Ajuste la opción Integración → Común si necesita preservar un `network.json` existente entre ejecuciones de Netzur.

## Integración con VISP

Primero, configure los parámetros relevantes de VISP en `/etc/lqos.conf`:

```ini
[visp_integration]
enable_visp = true
client_id = "su-client-id"
client_secret = "su-client-secret"
username = "usuario-app"
password = "password-app"
# Opcional: si se deja vacío, se usa el primer ISP ID del token
# isp_id = 0
timeout_secs = 20
# Opcional: dominio para enriquecimiento de sesiones online
# online_users_domain = ""
```

Notas:

- La importación VISP usa GraphQL y actualmente trabaja en estrategia `flat`.
- La integración reescribe `ShapedDevices.csv` en cada ejecución.
- `network.json` solo se sobrescribe cuando `always_overwrite_network_json = true` (en `[integration_common]`).
- Los tokens de VISP se cachean en `<lqos_directory>/.visp_token_cache_*.json`.

Importación manual:

```bash
python3 integrationVISP.py
```

Para ejecución automática con scheduler:

- `[visp_integration] enable_visp = true`
- reinicie scheduler:

```bash
sudo systemctl restart lqos_scheduler
```

## Integración con UISP

Primero, configure los parámetros relevantes de UISP en `/etc/lqos.conf`.

### Estrategias de Topología

LibreQoS soporta múltiples estrategias de topología para la integración con UISP para equilibrar el rendimiento del CPU con las necesidades de jerarquía de red:

| Estrategia | Descripción | Impacto en CPU | Caso de Uso |
|------------|-------------|----------------|-------------|
| `flat` | Solo regula suscriptores por velocidad del plan de servicio | Mínimo | Máximo rendimiento, regulación simple solo de suscriptores |
| `ap_only` | Regula por plan de servicio y Punto de Acceso | Bajo | Buen equilibrio entre rendimiento y control a nivel de AP |
| `ap_site` | Regula por plan de servicio, Punto de Acceso y Sitio | Medio | Agregación a nivel de sitio con complejidad moderada |
| `full` | Regula toda la red incluyendo backhauls, Sitios, APs y clientes | Alto | Mejor cuando topología/overrides ya están validados y estables |

**Cómo Elegir la Estrategia Correcta:**
- Comience con `ap_only` en despliegues nuevos y validación inicial
- Pase a `ap_site` cuando necesite control por sitio sin regulación de backhaul
- Pase a `full` cuando la calidad de topología y overrides esté validada y exista margen de CPU
- Use `flat` solo cuando el máximo rendimiento es crítico y no necesita ninguna jerarquía

**Nota de Rendimiento:** Cuando use la estrategia `full` con redes grandes, considere usar la función **promote_to_root** (vea [Promover Nodos a Raíz](#promover-nodos-a-raíz-optimización-de-rendimiento) arriba) para distribuir la carga del CPU entre múltiples núcleos.

### Estrategias de Manejo de Suspensiones

Configure cómo LibreQoS maneja las cuentas de clientes suspendidos:

| Estrategia | Descripción | Caso de Uso |
|------------|-------------|-------------|
| `none` | No manejar suspensiones | Cuando el manejo de suspensiones se gestiona en otro lugar |
| `ignore` | No agregar clientes suspendidos al mapa de red | Reduce el número de colas y mejora el rendimiento para redes con muchas cuentas suspendidas |
| `slow` | Limitar clientes suspendidos a 0.1 Mbps | Mantiene conectividad mínima para cuentas suspendidas (por ejemplo, portales de pago) |

**Cómo Elegir una Estrategia de Suspensión:**
- Use `none` si su router de borde u otro sistema maneja las suspensiones
- Use `ignore` para reducir la carga del sistema al no crear colas para clientes suspendidos
- Use `slow` para mantener conectividad mínima (útil para portales de pago o mensajes de servicio)

### Burst

- En UISP, las velocidades de Descarga y Subida (Download/Upload Speed) se configuran en Mbps (por ejemplo, 100 Mbps).
- En UISP, los valores de Burst de Descarga y Subida (Download/Upload Burst) se configuran en kilobytes por segundo (kB/s).
- Conversión y conformado:
  - burst_mbps = kB/s × 8 / 1000
  - Download Min = Download Speed (Mbps) × commit_bandwidth_multiplier
  - Download Max = (Download Speed (Mbps) + burst_mbps) × bandwidth_overhead_factor
  - Upload Min/Max se calculan igual desde Upload Speed (Mbps) y Upload Burst (kB/s)
- Ejemplo:
  - Valores en UISP: Download Speed = 100 Mbps, Download Burst = 12,500 kB/s
  - El burst añade 12,500 × 8 / 1000 = 100 Mbps
  - Download Min = 100 × commit_bandwidth_multiplier
  - Download Max = (100 + 100) × bandwidth_overhead_factor
- Referencia rápida (burst kB/s → Mbps añadidos):
  - 6,250 kB/s → +50 Mbps
  - 12,500 kB/s → +100 Mbps
  - 25,000 kB/s → +200 Mbps
- Notas:
  - Deje el burst vacío/nulo en UISP para desactivarlo.
  - Si suspended_strategy está en slow, Min y Max se fijan en 0.1 Mbps.

### Ejemplo de Configuración

```ini
[uisp_integration]
# Configuración Principal
enable_uisp = true
token = "su-token-api-aqui"
url = "https://uisp.su_dominio.com"
site = "Nombre_Sitio_Raiz"  # Sitio raíz para perspectiva de topología

# Estrategia de Topología (ver tabla arriba)
strategy = "ap_only"  # Punto de inicio recomendado para nuevos despliegues UISP

# Manejo de Suspensiones (ver tabla arriba)
suspended_strategy = "none"

# Ajustes de Capacidad
# Las capacidades de AP reportadas por UISP pueden ser optimistas
airmax_capacity = 0.8  # Usar 80% de la capacidad reportada de AirMax en instalaciones nuevas
airmax_flexible_frame_download_ratio = 0.8  # Reparto de respaldo para flexible framing de AirMax cuando UISP no expone dlRatio
ltu_capacity = 1.0      # Usar 100% de la capacidad reportada de LTU en instalaciones nuevas

# Gestión de Sitios
exclude_sites = []  # Sitios a excluir, ej: ["Sitio_Prueba", "Sitio_Lab"]
use_ptmp_as_parent = true  # Para sitios conectados a través de APs PtMP

# Ajustes de Ancho de Banda
bandwidth_overhead_factor = 1.15  # Dar a clientes 15% sobre velocidad del plan
commit_bandwidth_multiplier = 0.98  # Establecer mínimo al 98% del máximo (CIR)

# Opciones Avanzadas
ipv6_with_mikrotik = false  # Habilitar si usa DHCPv6 con MikroTik
always_overwrite_network_json = true  # Recomendado para despliegues guiados por integración
exception_cpes = []  # Excepciones de CPE en formato ["cpe:parent"]
squash_sites = []  # Opcional: sitios a compactar
enable_squashing = false  # Opcional: habilita lógica adicional de compactación
do_not_squash_sites = []  # Opcional: sitios excluidos de compactación
ignore_calculated_capacity = false  # Opcional: prioriza capacidad configurada sobre la calculada
insecure_ssl = false  # Opcional: ignora validación TLS del API UISP
```

### Opciones avanzadas/operativas de UISP

Las compilaciones actuales también incluyen estas opciones en los editores de configuración de WebUI (Node Manager):

- `exclude_sites`: lista de sitios a excluir de la importación.
- `exception_cpes`: sobrescrituras `cpe:parent` para asignación ambigua.
- `squash_sites`: lista opcional de sitios a compactar en flujos `full`.
- `enable_squashing`: habilita compactación adicional donde aplique.
- `do_not_squash_sites`: exclusiones explícitas de compactación.
- `use_ptmp_as_parent`: prioriza AP PtMP como padre en rutas relevantes.
- `ignore_calculated_capacity`: usa capacidades configuradas en lugar de calculadas.
- `insecure_ssl`: deshabilita validación de certificados TLS para UISP.
- `airmax_flexible_frame_download_ratio`: cuando UISP reporta capacidad agregada de un AP AirMax con flexible framing y no entrega `dlRatio`, LibreQoS usa esta proporción de descarga como respaldo. `0.8` significa 80/20 descarga/subida.

Las versiones actuales limitan este manejo de flexible framing a equipos donde UISP reporta `identification.type == "airMax"` y `identification.role == "ap"`. En esos AP AirMax, `theoreticalTotalCapacity` se usa solo como pista de flexible framing. La velocidad real de shaping sale de `totalCapacity` cuando UISP lo entrega, o de la capacidad direccional más fuerte cuando no lo hace, y la división sigue prefiriendo `dlRatio` cuando UISP lo reporta.

Uso recomendado:

1. Mantenga `insecure_ssl = false` salvo necesidad interna clara (PKI/self-signed).
2. Use primero `exclude_sites` y `do_not_squash_sites` para cambios más seguros.
3. Aplique `squash_sites`/`enable_squashing` de forma incremental y valide colocación de colas tras cada cambio.

Para probar la integración con UISP, ejecute:

```shell
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```
En la primera ejecución exitosa, se crearán los archivos network.json y ShapedDevices.csv.
Si ya existe un archivo network.json, no será sobrescrito a menos que configure ```always_overwrite_network_json = true```.

ShapedDevices.csv será sobrescrito cada vez que se ejecute la integración de UISP.

Si UISP expone un sitio y un AP con el mismo nombre visible dentro de la misma topología, las compilaciones actuales conservan el nombre del sitio en `network.json` y diferencian el nombre exportado del AP para que la rama del sitio no desaparezca del árbol.

Para asegurarse de que network.json siempre sea sobrescrito con la versión más reciente obtenida por la integración, edite el archivo `/etc/lqos.conf` con el comando `sudo nano /etc/lqos.conf` y configure el valor  `always_overwrite_network_json` a `true`.
Luego ejecute: `sudo systemctl restart lqosd`.

Tiene la opción de ejecutar `uisp_integration` automáticamente al iniciar el equipo y cada X minutos (configurado con el parámetro `queue_refresh_interval_mins`), lo cual es altamente recomendado. Esto se habilita estableciendo ```enable_uisp = true``` en `/etc/lqos.conf`. Una vez configurado, ejecute `sudo systemctl restart lqos_scheduler`.

### Sobrescrituras de UISP

Puede usar las siguientes entradas de override para reflejar su red con mayor precisión:
- `Rate Override` para cambios de ancho de banda a nivel de nodo guardados como entradas operatorias `AdjustSiteSpeed` en `lqos_overrides.json`
- `Topology Override` para correcciones de selección de padre en UISP `full`, también guardadas en `lqos_overrides.json`
- `uisp.route_overrides` en `lqos_overrides.json`
- integrationUISProutes.csv solo como entrada heredada de compatibilidad
- integrationUISPbandwidths.csv solo como entrada heredada de compatibilidad

Las compilaciones actuales aplican `Topology Override` antes de generar `network.json` y `ShapedDevices.csv`, y le dan precedencia sobre los overrides heredados de costos de ruta de UISP. El soporte actual en WebUI es solo `Pinned Parent`, forzando un único padre inmediato detectado.

Las compilaciones UISP actuales también auto-migran un `integrationUISPbandwidths.csv` heredado hacia overrides operatorios `AdjustSiteSpeed` en la siguiente ejecución de la integración cuando todavía no existen overrides de tasa del operador. Si ya existen, el CSV se ignora.
Las compilaciones UISP actuales también auto-migran un `integrationUISProutes.csv` heredado hacia `uisp.route_overrides` en `lqos_overrides.json` en la siguiente ejecución de la integración cuando todavía no existen overrides de ruta en JSON. Si ya existen overrides de ruta en JSON, el CSV se ignora.

Cada uno de los archivos mencionados arriba tienen plantillas, las cuales están disponibles en la carpeta `/opt/libreqos/src`. Si no los encuentra, puede obtenerlos [aquí](https://github.com/LibreQoE/LibreQoS/tree/develop/src). Para utilizarlos, copie el archivo (eliminando la parte `.template` del nombre) y edítelo con la información correspondiente.
Para cambios nuevos de ancho de banda, prefiera los overrides operatorios en `lqos_overrides.json`. `integrationUISPbandwidths.csv` queda como una entrada heredada para migración única, no como el flujo preferido a largo plazo.

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
