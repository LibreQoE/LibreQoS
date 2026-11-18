# Configurar el Puente de Regulación

## Elegir el Tipo de Puente

Hay dos opciones para que el puente pase datos entre las dos interfaces:

- Opción A: Puente Regular de Linux (Recomendado)
- Opción B: Puente Bifrost Acelerado con XDP

El puente regular de Linux es recomendado para la mayoría de las instalaciones. El puente de Linux continúa transfiriendo datos incluso si el servicio lqosd entra en un estado fallido, lo que lo hace una opción generalmente más segura para escenarios donde no hay una ruta de respaldo disponible. Funciona mejor con tarjetas de red Nvidia/Mellanox como las de la serie ConnectX-5 (que ofrecen un rendimiento superior en puentes) y configuraciones de máquinas virtuales con NICs virtualizadas. El puente Bifrost con XDP está recomendado para tarjetas de red Intel de 40G–100G que soportan XDP.

A continuación, se encuentran las instrucciones para configurar Netplan, ya sea usando el puente de Linux o el puente Bifrost con XDP:

```{note}
La página Network Mode de la interfaz web de LibreQoS inspecciona los archivos actuales de Netplan y ofrece interfaces elegibles que no forman parte de la ruta de gestión. Los modos puente de Linux e interfaz única pueden preparar y aplicar cambios administrados en `libreqos.yaml` con una ventana temporizada de reversión. El modo XDP guarda solamente `lqos.conf`; nunca genera ni aplica Netplan.
```

```{note}
La configuración inicial ofrece Puente Linux, Puente XDP e Interfaz Única. Para usar XDP con un bond, configure primero el bond en Netplan y seleccione su interfaz maestra en LibreQoS. No seleccione una interfaz miembro del bond.
```

```{note}
Si un cambio temporizado de Netplan interrumpe brevemente la sesión del navegador, vuelva a la página Network Mode cuando regrese la conectividad. LibreQoS retomará automáticamente desde esa página el flujo pendiente de confirmar o revertir.
```

## Opción A: Configuración de Netplan para un puente regular de Linux (Recomendado)

Ubuntu Server utiliza Netplan, el cual se basa en archivos .yaml ubicados en /etc/netplan para determinar la configuración de interfaces.
Aquí agregaremos un archivo .yaml específicamente para LibreQoS, de modo que no se sobrescriba cuando se hagan cambios al archivo .yaml por defecto.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Asumiendo que sus interfaces de regulación son ens19 y ens20, su archivo se vería así:

```yaml
network:
    ethernets:
        ens19:
            dhcp4: false
            dhcp6: false
        ens20:
            dhcp4: false
            dhcp6: false
    bridges:
        br0:
            interfaces:
            - ens19
            - ens20
    version: 2
```
```{note}
Asegurese de reemplazar `ens19` y `ens20` en el ejemplo anterior con las interfaces reales que utilizará para regular el tráfico. El orden de las interfaces no importa en esta sección.
```

Al establecer `dhcp4: false` y `dhcp6: false`, las interfaces de regulación se activarán como parte del ciclo normal de arranque, aunque no tengan direcciones IP asignadas.

Después ejecute:

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

## Opción B: Configuración de Netplan para el puente Bifrost con XDP

Ubuntu Server utiliza Netplan, el cual se basa en archivos .yaml ubicados en /etc/netplan para determinar la configuración de interfaces.
Aquí agregaremos un archivo .yaml específicamente para LibreQoS, de modo que no se sobrescriba cuando se hagan cambios al archivo .yaml por defecto.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Asumiendo que sus interfaces de regulación son ens19 y ens20, su archivo se vería así:

```yaml
network:
    ethernets:
        ens19:
            dhcp4: false
            dhcp6: false
        ens20:
            dhcp4: false
            dhcp6: false
    version: 2
```
```{note}
Asegurese de reemplazar `ens19` y `ens20` en el ejemplo anterior con las interfaces reales que utilizará para regular el tráfico. El orden de las interfaces no importa en esta sección.
```

Al establecer `dhcp4: false` y `dhcp6: false`, las interfaces de regulación se activarán como parte del ciclo normal de arranque, aunque no tengan direcciones IP asignadas.

Después ejecute:

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

Para usar el puente XDP, asegurese de establecer `use_xdp_bridge` como `true` en el archivo lqos.conf dentro de la sección [Configuración](configuration-es.md).

### Puente XDP con bonds 802.3ad

El controlador de bonding de Linux admite XDP nativo en interfaces maestras `802.3ad` cuando los controladores de todas las interfaces miembro también admiten XDP nativo. Configure LACP en los puertos conectados del switch y defina los bonds en Netplan antes de seleccionarlos en LibreQoS. El siguiente ejemplo usa un bond de dos puertos a cada lado del regulador:

```yaml
network:
    ethernets:
        enp1s0:
            dhcp4: false
            dhcp6: false
        enp2s0:
            dhcp4: false
            dhcp6: false
        enp3s0:
            dhcp4: false
            dhcp6: false
        enp4s0:
            dhcp4: false
            dhcp6: false
    bonds:
        bond-wan:
            interfaces: [enp1s0, enp2s0]
            parameters:
                mode: 802.3ad
        bond-lan:
            interfaces: [enp3s0, enp4s0]
            parameters:
                mode: 802.3ad
    version: 2
```

Después seleccione `bond-wan` como interfaz orientada a Internet y `bond-lan` como interfaz orientada a la LAN. La sección equivalente de `lqos.conf` es:

```toml
[bridge]
use_xdp_bridge = true
to_internet = "bond-wan"
to_network = "bond-lan"
```

No seleccione `enp1s0` a `enp4s0` en LibreQoS. Son miembros de los bonds; XDP debe adjuntarse a las interfaces maestras. Confirme que cada bond exponga varias colas RX/TX y que `lqosd` se adjunte en modo nativo del controlador antes de transportar tráfico de producción. La documentación del kernel de Linux enumera los [modos de bonding que admiten XDP nativo](https://docs.kernel.org/networking/bonding.html#what-bonding-modes-support-native-xdp).

Después de cambiar un nodo existente para usar las interfaces maestras de los bonds, reinicie `lqosd` para que adjunte XDP a las nuevas interfaces seleccionadas:

```shell
sudo systemctl restart lqosd
```
