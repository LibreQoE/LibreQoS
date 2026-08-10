# Configurar el Puente de Regulación

## Elegir el Tipo de Puente

Hay dos opciones para que el puente pase datos entre las dos interfaces:

- Opción A: Puente Regular de Linux (Recomendado)
- Opción B: Puente Bifrost Acelerado con XDP

El puente regular de Linux es recomendado para la mayoría de las instalaciones. El puente de Linux continúa transfiriendo datos incluso si el servicio lqosd entra en un estado fallido, lo que lo hace una opción generalmente más segura para escenarios donde no hay una ruta de respaldo disponible. Funciona mejor con tarjetas de red Nvidia/Mellanox como las de la serie ConnectX-5 (que ofrecen un rendimiento superior en puentes) y configuraciones de máquinas virtuales con NICs virtualizadas. El puente Bifrost con XDP está recomendado para tarjetas de red Intel de 40G–100G que soportan XDP.

A continuación, se encuentran las instrucciones para configurar Netplan, ya sea usando el puente de Linux o el puente Bifrost con XDP:

```{note}
La página Network Mode en la interfaz web de LibreQoS ahora inspecciona los archivos actuales de Netplan, ofrece en menús desplegables las interfaces elegibles que no forman parte de la ruta de gestión, prepara cambios administrados para `libreqos.yaml` en los modos puente de Linux y de interfaz única, los aplica con una ventana temporizada de reversión manejada por LibreQoS y le permite confirmar o revertir el cambio pendiente. También puede restaurar desde esa página la copia de seguridad administrada anterior. El modo puente XDP sigue siendo un flujo manual de Netplan.
```

```{note}
La configuración inicial actual solo ofrece Puente Linux e Interfaz Única para instalaciones nuevas. Si LibreQoS detecta un despliegue XDP existente, la configuración preserva ese modo XDP heredado y le avisa antes de migrar fuera de él.
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

## Capa de compatibilidad de interfaces

Use la capa de compatibilidad solo cuando LibreQoS no pueda adjuntar XDP directamente a las interfaces seleccionadas. Los casos más comunes son las interfaces enlazadas mediante bonding y los controladores de NIC que no ofrecen la compatibilidad con XDP que LibreQoS necesita. Si funciona XDP directo o el puente normal de Linux, esa opción sigue siendo preferible.

La capa conecta cada interfaz física a una interfaz veth multiqueue mediante un pequeño puente de Linux. LibreQoS adjunta a las interfaces veth su ruta existente de XDP y colas. Esto consume más CPU, pero mantiene activas en las interfaces físicas las descargas de sumas de comprobación, segmentación y VLAN. LibreQoS sigue aplicando los valores configurados de moderación de interrupciones cuando el controlador físico los acepta.

Actívela en la página `Bridge & Interface Mode`, o configure:

```toml
[bridge]
use_xdp_bridge = true
compatibility_shim = true
to_internet = "bond0"
to_network = "bond1"
```

Reinicie `lqosd` después de cambiar esta opción. LibreQoS elige la cantidad de colas veth según las CPU activas para shaping y cualquier override de colas configurado. Para la ruta de la capa usa el menor MTU de las dos interfaces físicas.

La capa de compatibilidad no añade a las interfaces físicas un límite de velocidad, un limitador HTB ni `fq_codel`. Configure el shaping de suscriptores mediante los ajustes normales de colas de LibreQoS.
