# Configurar LibreQoS

## Archivo de configuración principal
### /etc/lqos.conf

La configuración de LibreQoS para cada shaper box se almacena en el archivo `/etc/lqos.conf`.

Edite el archivo para que coincida con su configuración

```shell
sudo nano /etc/lqos.conf
```

En la sección ```[bridge]``` , cambie `to_internet` y `to_network` para que coincida con sus interfaces de red.
- `to_internet = "enp1s0f1"`
- `to_network = "enp1s0f2"`

Luego, si usa Bifrost/XDP, configure `use_xdp_bridge = true` en la misma sección `[bridge]`. Si no está seguro de que lo necesite, le recomendamos dejarlo como `false`.

- Configure downlink_bandwidth_mbps y uplink_bandwidth_mbps para que coincidan con el ancho de banda en Mbps de la conexión a internet de subida/WAN de su red. Puede hacer lo mismo con generated_pn_download_mbps y generated_pn_upload_mbps.
- to_internet sería la interfaz que da a su enrutador de borde y a la red de Internet en generla
- to_network sería la interfaz que da a su enrutador central (o red interna puenteada si su red está puenteada)

Nota: Si observa que el tráfico no se está modelando cuando debería, asegúrese de intercambiar el orden de las interfaces y reinicie lqosd, así como lqos_scheduler con ```sudo systemctl restart lqosd lqos_scheduler```.

Después de cambiar cualquier parte de `/etc/lqos.conf`, se recomienda encarecidamente reiniciar siempre lqosd, utilizando `sudo systemctl restart lqosd`. Esto vuelve a analizar cualquier nuevo valor en lqos.conf, haciendo que esos nuevos valores sean accesibles tanto para el lado Rust como para el lado Python del código.

### Integraciones

Más información sobre cómo [configurar integraciones aquí](../TechnicalDocs/integrations.md).

## Jerarquía de red
### Network.json

Network.json permite a los operadores de ISP definir una topología de red jerárquica o una topología de red plana.

Si planea utilizar las integraciones UISP, Splynx o Netzur integradas, no es necesario que cree un archivo network.json todavía.
Si planea utilizar la integración UISP integrada, esta se creará automáticamente la primera vez que se ejecute (suponiendo que network.json aún no exista).

Si no va a utilizar una integración, puede definir manualmente el archivo network.json siguiendo el archivo de plantilla: network.example.json

```text
+-----------------------------------------------------------------------+
| Entire Network                                                        |
+-----------------------+-----------------------+-----------------------+
| Parent Node A         | Parent Node B         | Parent Node C         |
+-----------------------+-------+-------+-------+-----------------------+
| Parent Node D | Sub 3 | Sub 4 | Sub 5 | Sub 6 | Sub 7 | Parent Node F |
+-------+-------+-------+-------+-------+-------+-------+-------+-------+
| Sub 1 | Sub 2 |       |                       |       | Sub 8 | Sub 9 |
+-------+-------+-------+-----------------------+-------+-------+-------+
```

Para redes sin nodos principales (sin puntos de acceso o sitios estrictamente definidos), edite el archivo network.json para utilizar una topología de red plana con
```nano network.json```
estableciendo el siguiente contenido del archivo:

```json
{}
```

#### Ayuda para la conversión de CSV a JSON

Puede utilizar

```shell
python3 csvToNetworkJSON.py
```

para convertir manualNetwork.csv en un archivo network.json.
manualNetwork.csv se puede copiar desde el archivo de plantilla, manualNetwork.template.csv.

Nota: El nombre del nodo principal debe coincidir con el utilizado para los clientes en ShapedDevices.csv.

## Circuitos

LibreQoS configura los dispositivos individuales según sus direcciones IP, que se agrupan en «circuitos».

Un circuito representa el servicio de Internet de un suscriptor de ISP, que puede tener una sola IP asociada (por ejemplo, al enrutador del suscriptor se le puede asignar una sola IPv4 /32) o puede tener varias IP asociadas (tal vez su enrutador tenga asignada una /29 o varias /32)..

LibreQoS sabe cómo configurar estos dispositivos y en qué nodo (AP, sitio, etc.) se encuentran, gracias al archivo ShapedDevices.csv.

### ShapedDevices.csv

El archivo ShapedDevices.csv correlaciona las direcciones IP de los dispositivos con los circuitos (el servicio exclusivo de cada suscriptor de Internet).

A continuación se muestra un ejemplo de una entrada en el archivo ShapedDevices.csv:
| Circuit ID | Circuit Name | Device ID | Device Name | Parent Node | MAC | IPv4                      | IPv6                 | Download Min Mbps | Upload Min Mbps | Download Max Mbps | Upload Max Mbps | Comment |
|------------|--------------|-----------|-------------|-------------|-----|---------------------------|----------------------|-------------------|-----------------|-------------------|-----------------|---------|
| 10001      | Person Name  | 10001     | Device 1    | AP_A        |     | 100.64.0.2, 100.64.0.8/29 | fdd7:b724:0:100::/56 | 25                | 5               | 155               | 20              |         |

Si utiliza una de nuestras integraciones CRM, este archivo se generará automáticamente. Si no utiliza una integración, puede editar el archivo manualmente utilizando la interfaz de usuario web o editando directamente el archivo ShapedDevices.csv a través de la CLI.

#### Edición manual mediante  WebUI
Navegue hasta el WebUI LibreQoS (http://a.b.c.d:9123) y seleccione Configuration > Shaped Devices.

#### Edición manual mediante CLI

- Modifique el archivo ShapedDevices.csv utilizando su editor de hojas de cálculo preferido (LibreOffice Calc, Excel, etc.), siguiendo la plantilla del archivo - ShapedDevices.example.csv
- Se requiere el ID del circuito. Debe ser una cadena de algún tipo (int está bien, se analiza como cadena). NO debe incluir ningún símbolo numérico (#). Cada circuito necesita un CircuitID único, no se pueden reutilizar. Aquí, circuito significa esencialmente la ubicación del cliente. Si un cliente tiene varias ubicaciones en diferentes partes de su red, utilice un CircuitID único para cada una de esas ubicaciones.
- Se requiere al menos una dirección IPv4 o IPv6 para cada entrada.
- El nombre del punto de acceso o del sitio debe configurarse en el campo Nodo principal. El campo Nodo principal puede dejarse en blanco para redes planas.
- El archivo ShapedDevices.csv le permite establecer el ancho de banda mínimo garantizado y máximo permitido por suscriptor.
- Las tarifas mínimas permitidas para los Circuitos son de 2 Mbit. El ancho de banda mínimo y máximo deben estar por encima de ese umbral.
- Recomendación: establezca el ancho de banda mínimo en algo así como 25/10 y el máximo en 1,15 veces la velocidad anunciada en el plan utilizando bandwidthOverheadFactor = 1,15
  - De esta manera, cuando un punto de acceso alcanza su límite máximo, los usuarios disponen de la capacidad restante del punto de acceso distribuida de forma equitativa entre ellos.
  - Garantizar un ancho de banda mínimo razonable para cada suscriptor, permitiéndoles utilizar hasta el máximo proporcionado cuando la utilización del punto de acceso sea inferior al 100 %.

Nota sobre los SLA: para los clientes con contratos SLA que les garantizan un ancho de banda mínimo, establezca la tarifa de su plan como el ancho de banda mínimo. De esta forma, cuando un punto de acceso se acerque a su límite máximo, los clientes con SLA siempre obtendrán esa cantidad.

![image](https://user-images.githubusercontent.com/22501920/200134960-28709d0f-48fe-4129-b4fd-70b204cade2c.png)

Una vez completada la configuración, ya está listo para ejecutar la aplicación e iniciar los [Deamons](./services-and-run.md)
