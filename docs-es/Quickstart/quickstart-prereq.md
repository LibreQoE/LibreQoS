# Configuración del servidor - Requisitos previos

Desactive el hyperthreading en la BIOS/UEFI de su sistema host. El hyperthreading también se conoce como multihilo simultáneo (SMT) en sistemas AMD. Desactivarlo es fundamental para optimizar el filtrado de CPUmap de XDP y, por consiguiente, el rendimiento y la latencia.

- Arranque, presionando la tecla apropiada para ingresar a la configuración del BIOS
- En sistemas AMD, deberá navegar por la configuración para encontrar la opción "Control SMT". Normalmente se encuentra en ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Una vez que lo encuentres, cambia a "Disabled" o "Off"
- Para los sistemas Intel, también deberá navegar por la configuración para encontrar la opción de "hyperthreading". En los servidores HP, se encuentra en ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options.```
- Guardar cambios y reiniciar

## Instalar en servidor Ubuntu

Recomendamos Ubuntu Server porque su versión de kernel suele coincidir estrechamente con las versiones principales de Linux. Nuestra documentación actual asume Ubuntu Server. Para ejecutar LibreQoS v1.4, se requiere el kernel de Linux 5.11 o superior, ya que incluye algunos parches importantes para XDP. Ubuntu Server 22.04 usa el kernel 5.13, que cumple con este requisito.

Puede descargar Ubuntu Server 22.04 desde <a href="https://ubuntu.com/download/server">https://ubuntu.com/download/server</a>.

1. Arrancar el servidor Ubuntu desde USB.
2. Siga los pasos para instalar Ubuntu Server.
3. Si usa una tarjeta de red Mellanox, el instalador de Ubuntu Server le preguntará si desea instalar los controladores de NIC Mellanox/Intel. Marque la casilla para confirmar. Este controlador adicional es importante.
4. En el paso de configuración de red, se recomienda asignar una dirección IP estática a la NIC de administración.
5. Asegúrese de que el servidor SSH esté habilitado para que pueda iniciar sesión en el servidor más fácilmente más tarde.
6. Puedes usar SCP o SFTP para acceder a archivos desde tu servidor LibreQoS y facilitar su edición. Aquí te explicamos cómo acceder mediante SCP o SFTP usando una máquina[Ubuntu](https://www.addictivetips.com/ubuntu-linux-tips/sftp-server-ubuntu/) o [Windows](https://winscp.net/eng/index.php).

### Elija el tipo de puente

Hay dos opciones para que el puente pase datos a través de sus dos interfaces:

- Puente acelerado Bifrost XDP
- Puente Linux regular

Se recomienda el puente Bifrost para NIC Intel con soporte XDP, como X520 y X710.Se recomienda el puente Linux normal para las NIC Nvidia/Mellanox como la serie ConnectX-5 (que tienen un rendimiento de puente superior) y configuraciones de VM que usan NIC virtualizadas.
Para utilizar el puente Bifrost, omita la sección del puente de Linux normal a continuación y asegúrese de habilitar Bifrost/XDP en lqos.conf algunas secciones a continuación.

### Agregar un puente Linux normal (si no se utiliza el puente Bifrost XDP)

Desde la máquina virtual Ubuntu, cree un puente de interfaz Linux (br0) con las dos interfaces de modelado.
Busque su archivo .yaml existente en /etc/netplan/ con

```shell
cd /etc/netplan/
ls
```

Luego edite el archivo .yaml allí con

```shell
sudo nano XX-cloud-init.yaml
```

Con XX correspondiente al nombre del archivo existente.

229 / 5.000
Al editar el archivo .yaml, necesitamos definir las interfaces de modelado (en este caso, ens19 y ens20) y agregar el puente con estas dos interfaces. Suponiendo que las interfaces sean ens18, ens19 y ens20, el archivo podría verse así:

```yaml
# This is the network config written by 'subiquity'
network:
  ethernets:
    ens18:
      addresses:
      - 10.0.0.12/24
      routes:
      - to: default
        via: 10.0.0.1
      nameservers:
        addresses:
        - 1.1.1.1
        - 8.8.8.8
        search: []
    ens19:
      dhcp4: no
    ens20:
      dhcp4: no
  version: 2
  bridges:
    br0:
      interfaces:
        - ens19
        - ens20
```

Asegúrese de reemplazar 10.0.0.12/24 con la dirección y subred de su VM LibreQoS, y de reemplazar la puerta de enlace predeterminada 10.0.0.1 con la que sea su puerta de enlace predeterminada.

Entonces ejecute

```shell
sudo netplan apply
```
