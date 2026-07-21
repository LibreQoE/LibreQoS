# Referencia avanzada de configuración

Use esta página cuando necesite configuración por CLI, edición directa de archivos o contenido de referencia detallado.

## Guardrails de patrones topológicos

Antes de ajustes profundos:

- Single-interface (on-a-stick): compatible, pero valide conteo de colas y mapeo por dirección tras cambios de interfaz/colas.
- Diseños con VLAN: compatibles si roles de interfaz y mapeo de nodos padre están bien definidos.
- En modo integración: no mantenga cambios permanentes en archivos que la integración regenera.

Si el resultado no coincide con lo esperado, use [Solución de problemas](troubleshooting-es.md) antes de seguir cambiando parámetros.

```{warning}
Si el modo integración está habilitado, las ediciones directas a `network.json` y `ShapedDevices.csv` pueden ser sobrescritas por los ciclos de refresco de integración. Use configuración de integración y overrides para cambios durables.
```

## Configuración por línea de comando

También puedes modificar ajustes usando la CLI.

### Archivo de Configuración Principal
### /etc/lqos.conf

La configuración para cada caja reguladora de LibreQoS se almacena en el archivo `/etc/lqos.conf`.

Edite el archivo para que coincida con su configuración usando:

```shell
sudo nano /etc/lqos.conf
```

En la sección ```[bridge]```, cambie `to_internet` y `to_network` para que coincidan con sus interfaces de red.
- `to_internet = "enp1s0f1"`
- `to_network = "enp1s0f2"`

En la sección `[bridge]` del archivo lqos.conf, puede habilitar o deshabilitar el puente XDP con la opción `use_xdp_bridge`. El valor predeterminado es `false` - ya que la configuración por defecto asume un [Puente Linux](prereq-es.md). Si eligió usar el puente XDP durante la configuración de requisitos previos, establezca `use_xdp_bridge = true`.

- Configure downlink_bandwidth_mbps y uplink_bandwidth_mbps para que coincidan con el ancho de banda en Mbps de la conexión WAN/Upstream de su red. Lo mismo puede hacerse para generated_pn_download_mbps y generated_pn_upload_mbps.
- to_internet es la interfaz que apunta hacia su router de borde (edge router) y el internet.
- to_network es la interfaz que apunta hacia su router interno (core router) (o la red interna puenteada, si su red está configurada de esa manera).

Nota: Si observa que el tráfico no se está regulando cuando debería, asegúrese de invertir el orden de las interfaces y reiniciar lqosd y lqos_scheduler con: ```sudo systemctl restart lqosd lqos_scheduler```.

Después de cambiar cualquier parte de `/etc/lqos.conf`, se recomienda reiniciar siempre lqosd usando: `sudo systemctl restart lqosd`. Esto integra los nuevos valores en lqos.conf, haciéndolos accesibles tanto para el código en Rust como en Python.

Logo compartido opcional:
- `display_cobrand` es un booleano opcional de nivel superior en `/etc/lqos.conf`.
- Si la clave no está presente, LibreQoS la trata como `false`.
- La WebUI solo muestra el logo del operador cuando `display_cobrand = true` y `cobrand.png` existe en el directorio de assets estáticos en runtime.

### Netflow (opcional)
Para habilitar Netflow, agregue la siguiente sección `[flows]` al archivo de configuración `/etc/lqos.conf`, configurando el `netflow_ip` adecuado:
```
[flows]
flow_timeout_seconds = 30
netflow_enabled = true
netflow_port = 2055
netflow_ip = "100.100.100.100"
netflow_version = 5
do_not_track_subnets = ["192.168.0.0/16"]
```

### Contabilidad RADIUS (opcional)

LibreQoS acepta una sección opcional `[radius_accounting]` para definir clientes NAS de confianza. Cuando está habilitada, `lqosd` inicia un servicio de contabilidad RADIUS, verifica paquetes de los clientes configurados, envía paquetes Accounting-Response para solicitudes aceptadas y mantiene el estado de sesión decodificado en memoria. Cuando `radius_accounting.dynamic_circuit_application.enabled` y la opción global `dynamic_circuits.enabled` están habilitadas, las sesiones Start e Interim-Update aptas se envían a la ruta de circuitos dinámicos.

Ejemplo:

```toml
[dynamic_circuits]
enabled = true

[radius_accounting]
enabled = true
listen = "0.0.0.0:1813"
default_ttl_seconds = 900
stale_grace_seconds = 120

[radius_accounting.dynamic_circuit_application]
enabled = true
match_shaped_devices_by_mac = true
match_shaped_devices_by_username = true
# Nodo padre opcional para identidades RADIUS por defecto cuando no se usan metadatos MAC.
# fallback_parent_node = "Core PPPoE"
# fallback_parent_node_id = "core-pppoe"
# fallback_anchor_node_id = "radius-anchor"

[radius_accounting.fallback_speed_profile]
download_min_mbps = 5.0
upload_min_mbps = 3.0
download_max_mbps = 25.0
upload_max_mbps = 10.0

[[radius_accounting.clients]]
name = "pppoe-core-1"
source = ["192.0.2.10/32"]
secret_file = "/etc/lqos/radius-secrets/pppoe-core-1"
```

Notas:
- Omita la sección o configure `enabled = false` para mantener deshabilitada la contabilidad RADIUS. Puede omitir los clientes mientras está deshabilitada; cualquier entrada de cliente que configure se sigue validando.
- Con `radius_accounting.dynamic_circuit_application.enabled = false`, los paquetes de contabilidad se aceptan y se rastrean, pero no cambian el shaping. Cuando está habilitado, LibreQoS puede resolver sesiones aptas como definiciones `ShapedDevice` en memoria. Los circuitos dinámicos solo se aplican si también está configurado `dynamic_circuits.enabled = true` en la sección global.
- Los paquetes Accounting-Response se envían de forma independiente de la aplicación de circuitos dinámicos. Si falla una creación o actualización, `lqosd` registra los identificadores de circuito y sesión y sigue escuchando.
- La aplicación de circuitos dinámicos crea o actualiza circuitos desde paquetes Start e Interim-Update aptos para shaping. Los paquetes Stop, la expiración por TTL y la expiración de sesiones obsoletas por reinicio NAS envían `RemoveDynamicCircuit` para el circuito activo creado por RADIUS. Los paquetes Accounting-Response se siguen enviando sin esperar esa eliminación asíncrona; los fallos se registran con los identificadores de circuito y sesión.
- Configure `radius_accounting.dynamic_circuit_application.match_shaped_devices_by_username = true` para comparar el valor RADIUS `User-Name` con una columna opcional `RADIUS Username` de `ShapedDevices.csv`. Una coincidencia única por nombre de usuario tiene prioridad sobre la coincidencia MAC, lo que permite sesiones PPPoE y DHCP-RADIUS sin requerir una dirección MAC.
- Configure `radius_accounting.dynamic_circuit_application.match_shaped_devices_by_mac = true` para comparar los valores RADIUS `Calling-Station-Id` con los valores MAC de `ShapedDevices.csv` cuando no haya una fila con el nombre de usuario. LibreQoS normaliza formatos con dos puntos, guiones, puntos, hexadecimal sin separadores y combinaciones de mayúsculas/minúsculas antes de comparar. Una coincidencia única aporta los campos de circuito, dispositivo, nodo padre, override SQM y velocidades de `ShapedDevices.csv`; las velocidades decodificadas del paquete tienen prioridad. Las direcciones IPv4 e IPv6 activas vienen de la sesión RADIUS. Las coincidencias duplicadas por nombre de usuario o MAC dejan la sesión pendiente.
- El servicio RADIUS carga los metadatos de coincidencia por nombre de usuario y MAC desde `ShapedDevices.csv` cuando inicia. Reinicie `lqosd` después de cambiar `RADIUS Username`, MAC, nodo padre, circuito, dispositivo, SQM o velocidad que las coincidencias RADIUS deban usar.
- Configure `fallback_parent_node` cuando las identidades RADIUS sin coincidencia deban quedar aptas para shaping mediante el perfil de velocidad de respaldo. Sin `fallback_parent_node`, las sesiones sin coincidencia quedan pendientes por falta de metadatos de nodo padre.
- `fallback_parent_node`, `fallback_parent_node_id` y `fallback_anchor_node_id` se usan solo para identidades dinámicas sin coincidencia. LibreQoS deriva su ID de circuito estable del NAS más el RADIUS `User-Name`, o del NAS más `Calling-Station-Id` cuando no hay nombre de usuario. `Acct-Session-Id` se usa solo para el ciclo de vida, por lo que los clientes que se reconectan conservan un único ID de circuito. Los paquetes de accounting sin ninguna de esas identidades de abonado quedan pendientes. Las sesiones con coincidencia conservan los metadatos de circuito y nodo padre de su fila de `ShapedDevices.csv`.
- Una sesión RADIUS solo queda apta para shaping cuando LibreQoS tiene una identidad estable de NAS más `Acct-Session-Id`, una identidad de dispositivo, al menos una dirección IP o prefijo recibido por RADIUS, metadatos de conexión a un nodo padre y un perfil de velocidad resuelto. Las sesiones sin metadatos de nodo padre quedan pendientes.
- Cualquier valor configurado en `listen` debe ser una dirección de escucha IP:puerto con un puerto distinto de cero, como `0.0.0.0:1813`. Cuando `enabled = true`, configure al menos un cliente. Cada cliente configurado debe incluir al menos una entrada `source`.
- `source` acepta una cadena IP/CIDR o una lista de cadenas IP/CIDR. Las direcciones IP sin prefijo se aceptan como hosts individuales.
- Cada cliente configurado debe incluir un `secret_file` no vacío. `lqosd` lee este archivo cuando inicia el servicio y usa su contenido como secreto compartido. LibreQoS conserva la ruta configurada en `/etc/lqos.conf`. La salida de depuración generada a partir de este campo oculta la ruta, pero los paquetes de soporte que incluyan `/etc/lqos.conf` pueden mostrar esa ruta.
- `default_ttl_seconds` y `stale_grace_seconds` deben ser mayores que cero.
- Omita `[radius_accounting.fallback_speed_profile]` cuando las sesiones sin una velocidad decodificada utilizable en el paquete RADIUS ni una velocidad de coincidencia MAC en `ShapedDevices.csv` deban quedar pendientes con motivo de velocidad faltante. Si una fila coincidente de `ShapedDevices.csv` contiene velocidades inválidas, la sesión queda pendiente en lugar de usar el perfil de respaldo.
- Cuando la aplicación de circuitos dinámicos de RADIUS está habilitada, los valores del perfil de velocidad de respaldo deben ser finitos y mayores que cero. `download_min_mbps` no debe superar `download_max_mbps`, y `upload_min_mbps` no debe superar `upload_max_mbps`.
- Reinicie `lqosd` después de cambiar esta sección para recargar el servicio y los archivos de secreto compartido.

### Integraciones con CRM/NMS

Más información sobre [configuración de integraciones aquí.](integrations-es.md).

### Límite de Fuente de Verdad para Usuarios de Integración

Si el modo integración está habilitado, los ciclos de refresco suelen ser dueños de `ShapedDevices.csv` y, según configuración, también de `network.json`.

- Use edición manual/WebUI para ajustes operativos temporales.
- Mantenga cambios permanentes en el sistema de integración, overrides de integración o flujo externo declarado.

### Modo de compilación de topología para archivos DIY/manuales

Para despliegues DIY/manuales que mantienen `network.json` y `ShapedDevices.csv`, use un modo que conserve jerarquía cuando los circuitos deban moldearse bajo los nombres `Parent Node` de `network.json`:

```toml
[topology]
compile_mode = "full"
```

Use `compile_mode = "flat"` solo cuando la jerarquía no forme parte del plan de shaping. En modo flat, LibreQoS asigna los circuitos a colas generadas por CPU, como `Generated_PN_1`; el `Parent Node` original queda como referencia lógica, pero el padre efectivo de shaping en `shaping_inputs.json` será una cola generada con `resolution_source: "flat_bucket"`.

### Overrides en runtime (`lqos_overrides.json`)

LibreQoS también permite ajustes de runtime mediante `lqos_overrides.json` en el `lqos_directory`.

```{mermaid}
flowchart LR
    A[CRM/NMS o archivos manuales] --> B[network.json + ShapedDevices.csv base]
    C[API/CLI de overrides] --> D[lqos_overrides.json]
    B --> E[Refresco de lqos_scheduler]
    D --> E
    E --> F[Plan de shaping combinado]
    F --> G[Colas/clases activas en lqosd]
```

## Jerarquía de Red
### Network.json

Network.json permite a los operadores de internet (ISP) definir una topología de red jerárquica o plana.

Cada nodo de topología puede incluir opcionalmente un campo `"id"`. Este campo está pensado para transportar un identificador estable del CRM/NMS de origen cuando exista. LibreQoS todavía sigue haciendo match de jerarquía y overrides por nombre de nodo, no por este ID, así que por ahora `"id"` es metadata.

Formato recomendado:

```json
{
  "Tower_A": {
    "id": "uisp:site:abc123",
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000,
    "type": "site"
  }
}
```

Notas:
- Use IDs string con namespace como `uisp:site:<id>`, `splynx:network_site:<id>` o `sonar:ap:<id>`.
- Los nodos generados solo por LibreQoS pueden usar IDs generados estables como `libreqos:generated:uisp:site:orphans`.
- También pueden aparecer campos de metadata específicos de integración como `uisp_site` y `uisp_device` junto con el campo genérico `id`.

Si planea usar las integraciones ya incluidas en LibreQoS de UISP, Splynx o Netzur, no necesita todavía crear un archivo network.json.
Si planea usar la integración ya incluida de UISP, el archivo network.json se creará en automático durante la primera ejecución (siempre y cuando network.json no exista previamente).

Si no planea usar una integración, puede definir manualmente el archivo network.json siguiendo el archivo de plantilla - [network.example.json](https://github.com/LibreQoE/LibreQoS/blob/develop/src/network.example.json). A continuación se muestra una ilustración en tabla del network.example.json. 

<table><thead><tr><th colspan="5">Entire Network</th></tr></thead><tbody><tr><td colspan="3">Site_1</td><td colspan="2">Site_2</td></tr><tr><td>AP_A</td><td colspan="2">Site_3</td><td>Pop_1</td><td>AP_1</td></tr><tr><td></td><td colspan="2">PoP_5</td><td>AP_7</td><td></td></tr><tr><td></td><td>AP_9</td><td>PoP_6</td><td></td><td></td></tr><tr><td></td><td></td><td>AP_11</td><td></td><td></td></tr></tbody></table>

Para redes sin nodos padre (sin puntos de acceso o sitios estrictamente definidos), edita el network.json para usar una topología de red plana con:
```
echo "{}" > network.json
```

#### Nodos virtuales (solo lógicos)

LibreQoS soporta **nodos virtuales** en `network.json` para agrupar y para monitoreo/agregación en la WebUI/Insight. Los nodos virtuales **no** se incluyen en el árbol físico de shaping HTB (no crean clases HTB y no aplican límites de ancho de banda).

```{mermaid}
flowchart TD
    A[Árbol lógico incluye nodo virtual] --> B[Fase de armado del scheduler]
    B --> C[Promover hijos virtuales al ancestro no virtual más cercano]
    C --> D{¿Colisión de nombres entre hermanos tras promoción?}
    D -->|No| E[Se genera el árbol físico de shaping]
    D -->|Sí| F[Error de build: renombrar/reestructurar nodos]
```

Para marcar un nodo como virtual, configura `"virtual": true` en ese nodo.

Ejemplo:

```json
{
  "Region": {
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000,
    "children": {
      "Town": {
        "virtual": true,
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 500,
        "children": {
          "AP_A": {
            "downloadBandwidthMbps": 200,
            "uploadBandwidthMbps": 200
          }
        }
      }
    }
  }
}
```

Notas:
- Durante el shaping, los nodos virtuales se eliminan y sus hijos se promueven al ancestro no virtual más cercano (revisa `queuingStructure.json` para el plan físico activo).
- `ShapedDevices.csv` aún puede usar un nodo virtual como `Parent Node` para mostrar/agrupar; LibreQoS adjuntará esos circuitos para shaping al ancestro no virtual más cercano (si el nodo virtual está en el nivel superior y no tiene ancestro no virtual, se tratará como sin nodo padre para el shaping).
- Evita colisiones de nombres después de la promoción (dos nodos con el mismo nombre terminando al mismo nivel).
- Queue Auto también puede virtualizar ramas de alta capacidad de agregación, uplink o AP representadas como nodos `Site` o `AP` cuando superan `queue_auto_virtualize_threshold_mbps` y tienen ramas de cola hijas. Esto evita que una rama lógica grande se convierta en un cuello de botella de una sola cola de CPU.

#### Consideraciones de CPU

La planificación de CPU debe seguir el **árbol físico de shaping** (después de promoción), no el árbol lógico crudo de `network.json`.

```{mermaid}
flowchart LR
    A[Topología lógica en network.json<br/>puede incluir nodos virtuales] --> B[Promoción durante el armado<br/>remover virtuales y promover hijos]
    B --> C[Árbol físico HTB<br/>solo nodos reales]
    C --> D[Asignación CPU/binpacking<br/>distribuir nodos físicos de primer nivel]
```

```{mermaid}
flowchart LR
    subgraph Arbol Logico
        L1[Region]
        L2[Town virtual]
        L3[AP_A]
        L4[AP_B]
        L1 --> L2
        L2 --> L3
        L2 --> L4
    end
    subgraph Arbol HTB Fisico
        P1[Region]
        P2[AP_A]
        P3[AP_B]
        P1 --> P2
        P1 --> P3
    end
```

```{mermaid}
flowchart TD
    WAN[Objetivo WAN modelado 20 Gbps ejemplo] --> C1[CPU 1 presupuesto seguro ~5 Gbps]
    WAN --> C2[CPU 2 presupuesto seguro ~5 Gbps]
    WAN --> C3[CPU 3 presupuesto seguro ~5 Gbps]
    WAN --> C4[CPU 4 presupuesto seguro ~5 Gbps]
    C1 --> N1[Nodos fisicos de primer nivel asignados aqui]
    C2 --> N2[Nodos fisicos de primer nivel asignados aqui]
    C3 --> N3[Nodos fisicos de primer nivel asignados aqui]
    C4 --> N4[Nodos fisicos de primer nivel asignados aqui]
```

Notas:
- Los nodos virtuales son solo lógicos y no crean clases HTB.
- La asignación de CPU/binpacking trabaja sobre el árbol físico post-promoción.
- Si la promoción produce colisiones de nombres entre hermanos, el build falla.
- Los valores por núcleo de arriba son ejemplos de planificación, no límites codificados.

#### Ayudante para convertir CSV a JSON

Puede usar:

```shell
python3 csvToNetworkJSON.py
```

para convertir manualNetwork.csv en un archivo network.json.
manualNetwork.csv puede copiarse desde el archivo de plantilla manualNetwork.template.csv.

Nota: El nombre del nodo padre debe coincidir con el nombre usado para los clientes en ShapedDevices.csv.

## Circuitos

LibreQoS regula dispositivos individuales por sus direcciones IP, las cuales son agrupadas en "circuitos".

Un circuito representa el servicio de internet de un suscriptor de la operadora de internet, el cual puede tener solo una IP asociada (por ejemplo, el router del suscriptor puede tener asignada una sola IPv4 /32) o puede tener asignado varias IPs (quizás su router tenga asignado un /29, o varias /32).

El archivo ShapedDevices.csv correlaciona direcciones IP de dispositivos con circuitos (el servicio único de cada suscriptor de internet).

### ShapedDevices.csv

El archivo ShapedDevices.csv correlaciona las direcciones IP de los dispositivos con los circuitos (cada servicio único de suscriptor).

A continuación un ejemplo de entrada en ShapedDevices.csv:
| Circuit ID | Circuit Name                                        | Device ID | Device Name | Parent Node | MAC | IPv4                    | IPv6                 | Download Min Mbps | Upload Min Mbps | Download Max Mbps | Upload Max Mbps | Comment |
|------------|-----------------------------------------------------|-----------|-------------|-------------|-----|-------------------------|----------------------|-------------------|-----------------|-------------------|-----------------|---------|
| 1          | 968 Circle St., Gurnee, IL 60031                    | 1         | Device 1    | AP_A        |     | 100.64.0.1, 100.64.0.14 | fdd7:b724:0:100::/56 | 1                 | 1               | 155               | 20              |         |
| 2          | 31 Marconi Street, Lake In The Hills, IL 60156      | 2         | Device 2    | AP_A        |     | 100.64.0.2              | fdd7:b724:0:200::/56 | 1                 | 1               | 105               | 18              |         |
| 3          | 255 NW. Newport Ave., Jamestown, NY 14701           | 3         | Device 3    | AP_9        |     | 100.64.0.3              | fdd7:b724:0:300::/56 | 1                 | 1               | 105               | 18              |         |
| 4          | 8493 Campfire Street, Peabody, MA 01960             | 4         | Device 4    | AP_9        |     | 100.64.0.4              | fdd7:b724:0:400::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 5         | Device 5    | AP_11       |     | 100.64.0.5              | fdd7:b724:0:500::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 6         | Device 6    | AP_11       |     | 100.64.0.6              | fdd7:b724:0:600::/56 | 1                 | 1               | 105               | 18              |         |
| 5          | 93 Oklahoma Ave., Parsippany, NJ 07054              | 7         | Device 7    | AP_1        |     | 100.64.0.7              | fdd7:b724:0:700::/56 | 1                 | 1               | 155               | 20              |         |
| 6          | 74 Bishop Ave., Bakersfield, CA 93306               | 8         | Device 8    | AP_1        |     | 100.64.0.8              | fdd7:b724:0:800::/56 | 1                 | 1               | 105               | 18              |         |
| 7          | 9598 Peg Shop Drive, Lutherville Timonium, MD 21093 | 9         | Device 9    | AP_7        |     | 100.64.0.9              | fdd7:b724:0:900::/56 | 1                 | 1               | 105               | 18              |         |
| 8          | 115 Gartner Rd., Gettysburg, PA 17325               | 10        | Device 10   | AP_7        |     | 100.64.0.10             | fdd7:b724:0:a00::/56 | 1                 | 1               | 105               | 18              |         |
| 9          | 525 Birchpond St., Romulus, MI 48174                | 11        | Device 11   | Site_1      |     | 100.64.0.11             | fdd7:b724:0:b00::/56 | 1                 | 1               | 105               | 18              |         |

Si está utilizando una de nuestras integraciones con CRM, este archivo se generará automáticamente. Si no está utilizando una integración, puede editar el archivo manualmente usando la interfaz WebUI o editando directamente el archivo ShapedDevices.csv a través de la CLI.

#### TreeGuard y SQM por circuito

TreeGuard puede ajustar dinámicamente el SQM por circuito (`cake`/`fq_codel`) según condiciones del circuito.

Importante:
- En LibreQoS v2.0, TreeGuard está habilitado por defecto.
- Si desea un comportamiento SQM fijo/manual, revise la configuración de TreeGuard temprano en el despliegue y ajuste su enrolamiento o deshabilítelo explícitamente.

Consulte [TreeGuard](treeguard-es.md).

#### Múltiples IPs por Circuito
Si necesita listar múltiples IPv4 en el campo IPv4, o múltiples IPv6 en el campo IPv6, agregue una coma entre ellas. Si está editando con un editor CSV (LibreOffice Calc, Excel), el editor CSV automáticamente colocará comillas alrededor de los elementos separados por coma. Si usted está editando el archivo manualmente con un editor de texto como notepad o nano, por favor agregue comillas alrededor de las entradas separadas por coma.

```
2794,"6 Littleton Drive, Ringgold, GA 30736",5,Device 5,AP_11,,100.64.0.5,"fdd7:b724:0:500::/56,fdd7:b724:0:600::/56",1,1,105,18,""
```

#### Edición Manual mediante WebUI
Navegue a la interfaz Web de LibreQoS (http://a.b.c.d:9123) y seleccione Configuration > Shaped Devices.

#### Edición Manual mediante CLI

- Modifique el archivo ShapedDevices.csv usando su editor de hojas de cálculo preferido (LibreOffice Calc, Excel, etc.), siguiendo el archivo de plantilla - ShapedDevices.example.csv.
- El Circuit ID es obligatorio. El Circuit ID puede ser un número o texto. Este campo NO debe incluir símbolos de hashtag (#). Cada circuito requiere un Circuit ID único – no pueden reutilizarse. Aquí, "circuito" se refiere esencialmente al servicio de un cliente. Si un cliente tiene múltiples ubicaciones en diferentes partes de su red, use un Circuit ID único para cada una.
- Se requiere al menos una dirección IPv4 o IPv6 para cada entrada.
- El nombre del Punto de Acceso o Sitio debe establecerse en el campo Parent Node. Puede dejarse en blanco para redes planas.
- El archivo ShapedDevices.csv le permite establecer el ancho de banda mínimo (garantizado) y el máximo permitido por suscriptor.
- Los valores Download Min y Upload Min de cada circuito deben ser 1 Mbps o mayores. Generalmente, deberían configurarse en 1 Mbps por defecto.
- Los valores Download Max y Upload Max de cada circuito deben ser 2 Mbps o mayores. Generalmente, corresponden al plan de velocidad contratado por el cliente.
- Recomendación: configure el ancho de banda mínimo en 1/1 y el máximo en 1.15X la velocidad anunciada del plan:
  - De esta manera, cuando un AP alcance su límite, los usuarios tendrán la capacidad restante del AP distribuida equitativamente.
  - Al establecer el máximo en 1.15X la velocidad contratada, se incrementa la probabilidad de que el suscriptor vea un resultado satisfactorio en un test de velocidad, incluso si hay tráfico ligero en segundo plano en su circuito (como un video HD, actualizaciones de software, etc.).
  - Esto permite a los suscriptores usar hasta el limite máximo de su plan cuando el AP tiene capacidad disponible.

Nota sobre SLAs: Para clientes con contratos SLA, donde garantizan un ancho de banda mínimo, puede configurar el plan contratado como el ancho de banda mínimo. De esta manera, cuando un AP se acerque a su límite, los clientes con SLA siempre tendrán esa velocidad disponible. Asegúrese de que la suma de los anchos de banda mínimos de los circuitos conectados a un nodo padre no supere la capacidad total de ese nodo padre. Si esto ocurre, LibreQoS tiene un mecanismo de seguridad que [reduce los mínimos a 1/1](https://github.com/LibreQoE/LibreQoS/pull/643) para todos los circuitos afectados. 

Una vez que su configuración esté completa, estará listo para ejecutar la aplicación e iniciar los [servicios systemd](./components-es.md#servicios-de-systemd)
