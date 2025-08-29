# Configurar LibreQoS

## Archivo de Configuración Principal
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

### Netflow (optional)
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

Si planea usar las integraciones ya incluidas en LibreQoS de UISP o Splynx, no necesita todavía crear un archivo network.json.
Si planea usar la integración ya incluida de UISP, el archivo network.json se creará en automático durante la primera ejecución (siempre y cuando network.json no exista previamente).

Si no planea usar una integración, puede definir manualmente el archivo network.json siguiendo el archivo de plantilla - [network.example.json](https://github.com/LibreQoE/LibreQoS/blob/develop/src/network.example.json). A continuación se muestra una ilustración en tabla del network.example.json. 

<table><thead><tr><th colspan="5">Entire Network</th></tr></thead><tbody><tr><td colspan="3">Site_1</td><td colspan="2">Site_2</td></tr><tr><td>AP_A</td><td colspan="2">Site_3</td><td>Pop_1</td><td>AP_1</td></tr><tr><td></td><td colspan="2">PoP_5</td><td>AP_7</td><td></td></tr><tr><td></td><td>AP_9</td><td>PoP_6</td><td></td><td></td></tr><tr><td></td><td></td><td>AP_11</td><td></td><td></td></tr></tbody></table>

Para redes sin nodos padre (sin puntos de acceso o sitios estrictamente definidos), edite el network.json para usar una topología de red plana con:
```
echo "{}" > network.json
```

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

The ShapedDevices.csv file correlates device IP addresses to Circuits (each internet subscriber's unique service).

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
