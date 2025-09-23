# Configure LibreQoS

## Configuración de lqos.conf

Copie la configuración archivo del demonio lqosd en `/etc`:

```shell
cd /opt/libreqos/src
sudo cp lqos.example /etc/lqos.conf
```

Ahora edite el archivo para igualar su configuración con

```shell
sudo nano /etc/lqos.conf
```

Cambie `enp1s0f1` y `enp1s0f2` para coincidir con sus interfaces de red. No importa cuál sea cuál. Tenga en cuenta que está emparejando las interfaces, por lo que cuando ingresa por primera vez  enps0f<ins>**1**</ins>  en la primera línea, el parámetro es enp1s0f<ins>**2**</ins> (reemplace con los nombres de interfaz reales).

- Primera Línea: `name = "enp1s0f1", redirect_to = "enp1s0f2"`
- Segunda Línea: `name = "enp1s0f2", redirect_to = "enp1s0f1"`

Luego, si estará usando Bifrost/XDP, configure `use_xdp_bridge = true` bajo la misma sección `[bridge]`.

## Configuración de ispConfig.py

Copie ispConfig.example.py a ispConfig.py y edite según necesidad

```shell
cd /opt/libreqos/src/
cp ispConfig.example.py ispConfig.py
nano ispConfig.py
```

- Configure upstreamBandwidthCapacityDownloadMbps y upstreamBandwidthCapacityUploadMbps para que coincidan con el ancho de banda de subida de su red/WAN o conexión a Internet. Lo mismo puede hacerse para generatedPNDownloadMbps y generatedPNUploadMbps.
- Estableza la interfaceA en la interfaz orientada a su router principal (o red interna en puente si su red está en esa modalidad)
- Estableza la interfaceB en la interfaz orientada hacia su enrutador de borde.
- Establezca ```enableActualShellCommands = True```  para permitir que el programa ejecute los comandos propiamente tal.

## Network.json

Network.json permite a los operadores de ISP definir una Topología de Red Jerárquica, o una Topología de Red Plana.

Para redes sin Nodos Principales (sin Puntos de Acceso ni Sitios definidos estrictamentens) edite el archivo network.json para usar una Topología de Red Plana con
```nano network.json```
configurando el siguiente contenido de archivo:

```json
{}
```

Si planea usar las integraciones UISP, Splynx o Netzur incorporadas, todavía no es necesario crear un archivo network.json.

Si planea usar las integraciones UISP incorporadas, este se creará automáticamente en su primera ejecución (asumiendo que  network.json no esté ya presente). Después puede modificar network.json para que refleje más ajustadamente su topología.

Si no usará una integración, puede definirlo manualmente en  network.json siguiendo el archiv plantilla - network.example.json

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

## Configuración Manual

Usted puede usar

```shell
python3 csvToNetworkJSON.py
```

Para convertir manualNetwork.csv como un archivo network.json.
manualNetwork.csv puede ser copiado desde un archivo plantilla, manualNetwork.template.csv

Nota: El nombre del nodo principal debe coincidir con el utilizado para los clientes en ShapedDevices.csv

## ShapedDevices.csv

Si está realizando una integración, este archivo será generado automáticamente. Si no está haciendo una integración, puede editar manualmente el archivo.

### Edición Manual

- Modifique el archivo ShapedDevices.csv su editor de planillas electrónicas preferido (LibreOffice Calc, Excel, etc), siguiendo el archivo de plantilla - ShapedDevices.example.csv
- Se requiere ID Circuit. Este debe ser una cadena de algún tipo (un entero es suficiente, se analiza como cadena). NO debe incluir símbolos numéricos (#). Cada circuito necesita un CircuitID único, el que no se puede reutilizar. Aquí, un circuito significa básicamente la ubicación del cliente. Si un cliente tiene múltiples ubicaciones en diferentes partes de la red, utilice un ID de circuito único para cada una de ellas.
- Al menos una dirección IPv4 o IPv6 se requiere para cada entrada.
- El nombre del Punto de Acceso o Sitio debe configurarse en el campo de "Nodo Principal". Este campo puede quedar en blanco para redes planas.
- El archivo ShapedDevices.csv permite establecer un ancho de banda mínimo garantizado y un máximo permitido por suscriptor.
- La velocidad mínima permitida para los circuitos es de 2 Mbit. El ancho de banda mínimo de ser superior a ese umbral.
- Recomendación: establezca el ancho de banda mínimo en algo así como 25/10 y un máximo de 1.15X en base a la tarifa del plan publicitado utilizando  bandwidthOverheadFactor = 1.15
  - De esta manera, cuando un AP alcanza su límite, los usuarios tienen toda la capacidad de AP restante distribuida equitativamente entre ellos.
  - Garantice un ancho de banda mínimo para cada suscriptor, permitiéndoles utilizar el máximo proporcionado cuando la utilización del AP sea inferior al 100%.

Nota acerca de los SLAs: Para los clientes que tienen contratos de  SLA que les garanticen un ancho de banda mínimo, establezcan la tarifa de su plan como el ancho de banda mínimo. De esta manera, cuando un punto de acceso se acerque a su límite, los clientes con SLA siempre recibirán esa cantidad.

![image](https://user-images.githubusercontent.com/22501920/200134960-28709d0f-48fe-4129-b4fd-70b204cade2c.png)

Una vez completada la configuración, estará listo para ejecutar la aplicación e iniciar  [Deamons](./services-and-run.md)
