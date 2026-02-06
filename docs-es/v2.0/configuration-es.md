# Configurar LibreQoS

## Configuración inicial mediante la herramienta de instalación (desde el .deb)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

NOTAS:
- La herramienta de configuración de LibreQoS solo se controla con teclado. Usa las flechas para desplazarte y presiona ```Enter``` para seleccionar.
- Al presionar la tecla ```Q``` se cierra la herramienta sin guardar.
- Si estabas usando la herramienta y se cerró, debes ejecutar los siguientes comandos para relanzarla:
  ```
  sudo apt remove libreqos
  sudo apt install ./{deb_url_v1_5}
  ```

El primer paso es darle un nombre a tu Shaper Box (nodo). Por defecto se llama LibreQoS.
Usa las flechas del teclado para recorrer las secciones de configuración:

### Bridge Mode
<img width="1668" height="397" alt="bridge_mode" src="https://github.com/user-attachments/assets/22bc05cc-f1e5-451a-b4f8-5e75d7b8d64f" />

De manera predeterminada se selecciona el puente Linux. Si elegiste el puente XDP en el paso anterior, ajústalo aquí.

NOTA: el modo Single Interface, como su nombre indica, es para usuarios que solo pueden usar 1 interfaz y requiere soporte especial. Para más detalles visita nuestro [Zulip Chat].

### Interfaces
<img width="885" height="352" alt="interfaces" src="https://github.com/user-attachments/assets/4afedfe6-65b8-450c-a675-bea25ef4553c" />

Siguiendo el diagrama recomendado, la interfaz “To Internet” debe apuntar al router de borde (y, por lo tanto, a Internet) y la interfaz “To Network” debe apuntar al router central.

### Bandwidth
<img width="1089" height="350" alt="bandwidth" src="https://github.com/user-attachments/assets/f68185c3-82dc-4fb5-b78a-d812665533fb" />

En el contexto del ancho de banda contratado a tu proveedor upstream, ```To Internet``` representa el ancho de banda de subida y ```To Network``` el de bajada.

### IP Range
<img width="1331" height="481" alt="ip_ranges" src="https://github.com/user-attachments/assets/b846baa7-288e-460c-ab77-ad400384057c" />

En esta sección debes especificar todos los rangos IP utilizados por los routers de tus clientes, incluyendo rangos para IPs estáticas. De forma predeterminada se incluyen 4 rangos comunes, como se ve en la imagen.

*Tip: para eliminar un rango, selecciónalo con las flechas del teclado y presiona ```Tab``` hasta resaltar ```<Remove Selected>```, luego ```Enter```.

### Web Users
<img width="1664" height="528" alt="web_users" src="https://github.com/user-attachments/assets/8db17e0e-cc3d-4d67-9c59-4751bc4d9b0f" />

Aquí puedes crear usuarios para el panel web de LibreQoS con sus respectivos roles. Hay dos tipos: ```Admin``` y ```Read Only```.
***
Después de guardar el archivo podrías ver el siguiente mensaje en la terminal:
```
No VM guests are running outdated hypervisor (qemu) binaries on this host.
N: Download is performed unsandboxed as root as file '/home/libreqos/libreqos_1.5-RC2.202510052233-1_amd64.deb' couldn't be accessed by user '_apt'. - pkgAcquire::Run (13: Permission denied)
```
Ese mensaje es benigno; puedes ignorarlo.

### Próximos pasos

Si la instalación se completó correctamente, podrás ingresar a la WebUI en ```http://tu_ip_del_shaper:9123```. En el primer ingreso podrás definir usuario y contraseña.

Luego, configura tu [integración con CRM o NMS](integrations-es.md) usando la página de configuración de la WebUI. Si no usas un CRM/NMS soportado, tendrás que crear un script o proceso que genere los archivos necesarios para que LibreQoS pueda regular el tráfico: `network.json` y `ShapedDevices.csv`. Los formatos se explican más adelante en esta página.

## Configuración mediante la interfaz web

La mayoría de los parámetros del shaper pueden modificarse desde la página Configuration en la WebUI (http://tu_ip_del_shaper:9123/config_general.html).

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

### Integraciones con CRM/NMS

Más información sobre [configuración de integraciones aquí.](integrations-es.md).

## Jerarquía de Red
### Network.json

Network.json permite a los operadores de internet (ISP) definir una topología de red jerárquica o plana.

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

Para marcar un nodo como virtual, configura `"virtual": true` en ese nodo. (Compatibilidad heredada: `"type": "virtual"` también se reconoce, pero se recomienda `"virtual": true` para poder mantener un `"type"` real como `"Site"` o `"AP"`.)

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

#### Consideraciones de CPU

<img width="3276" height="1944" alt="cpu" src="https://github.com/user-attachments/assets/e4eeed5e-eeeb-4251-9258-d301c3814237" />

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
