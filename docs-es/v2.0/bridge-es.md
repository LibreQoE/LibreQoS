# Configurar el Puente de Regulación

## Elegir el Tipo de Puente

Hay dos opciones para que el puente pase datos entre las dos interfaces:

- Opción A: Puente Regular de Linux (Recomendado)
- Opción B: Puente Bifrost Acelerado con XDP

El puente regular de Linux es recomendado para la mayoría de las instalaciones. El puente de Linux continúa transfiriendo datos incluso si el servicio lqosd entra en un estado fallido, lo que lo hace una opción generalmente más segura para escenarios donde no hay una ruta de respaldo disponible. Funciona mejor con tarjetas de red Nvidia/Mellanox como las de la serie ConnectX-5 (que ofrecen un rendimiento superior en puentes) y configuraciones de máquinas virtuales con NICs virtualizadas. El puente Bifrost con XDP está recomendado para tarjetas de red Intel de 40G–100G que soportan XDP.

A continuación, se encuentran las instrucciones para configurar Netplan, ya sea usando el puente de Linux o el puente Bifrost con XDP:

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
            dhcp4: no
            dhcp6: no
        ens20:
            dhcp4: no
            dhcp6: no
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

Al establecer `dhcp4: no` y `dhcp6: no`, las interfaces de regulación se activarán como parte del ciclo normal de arranque, aunque no tengan direcciones IP asignadas.

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
            dhcp4: no
            dhcp6: no
        ens20:
            dhcp4: no
            dhcp6: no
    version: 2
```
```{note}
Asegurese de reemplazar `ens19` y `ens20` en el ejemplo anterior con las interfaces reales que utilizará para regular el tráfico. El orden de las interfaces no importa en esta sección.
```

Al establecer `dhcp4: no` y `dhcp6: no`, las interfaces de regulación se activarán como parte del ciclo normal de arranque, aunque no tengan direcciones IP asignadas.

Después ejecute:

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

Para usar el puente XDP, asegurese de establecer `use_xdp_bridge` como `true` en el archivo lqos.conf dentro de la sección [Configuración](configuration-es.md).