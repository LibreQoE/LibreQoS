# Requisitos previos para la configuración del servidor

## Servidores e hipervisores

### Desactivar Hyper-Threading
Desactiva el Hyper-Threading en la BIOS/UEFI de tu sistema host. El Hyper-Threading también se conoce como Simultaneous Multi Threading (SMT) en sistemas AMD. Desactivarlo es muy importante para obtener un rendimiento óptimo del filtrado cpumap con XDP y, por ende, mejorar el rendimiento y la latencia.

- Inicia el sistema y presiona la tecla correspondiente para entrar a la configuración de la BIOS.
- En sistemas AMD, deberás navegar por las opciones hasta encontrar la opción llamada "SMT Control". Normalmente se encuentra en: ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Una vez localizada, cambia la opción a "Disabled" o "Off".
- En sistemas Intel, también deberás buscar la opción para desactivar el Hyper-Threading en ajustes. En servidores HP, por ejemplo, está ubicada en: ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options```
- Guarda los cambios y reinicia el sistema.

### Desactivar SR-IOV en la BIOS

SR-IOV puede desactivar XDP nativo (modo controlador) en las Funciones Físicas (PF), forzando XDP Genérico (SKB) y reduciendo el rendimiento y la estabilidad de LibreQoS. Desactive SR-IOV en la BIOS/UEFI para las tarjetas de red (NIC) utilizadas por LibreQoS. Si existen opciones por ranura/por puerto, desactívelas.

## Hipervisores

### Proxmox

Para las máquinas virtuales Proxmox, se requiere el paso a través de la tarjeta de red (NIC) para alcanzar un rendimiento superior a 10 Gbps (XDP vs. XDP genérico).

LibreQoS requiere dos o más colas RX/TX, por lo que al usar Proxmox, asegúrese de habilitar [Multicola](https://forum.proxmox.com/threads/where-is-multiqueue.146783/) para las interfaces de modelado asignadas a la máquina virtual. La multicola debe tener un valor igual al número de núcleos de vCPU asignados a la máquina virtual.

### Hyper-V

#### Suplantación de MAC en Hyper-V (Si está dentro de una máquina virtual)

Si su sistema LibreQoS se ejecuta dentro de Hyper-V y ha conectado dos vNIC (eth1, eth2), Hyper-V bloqueará el tráfico del puente porque este genera tramas con direcciones MAC diferentes a la asignada a la vNIC. Para solucionar esto en un host Windows:
```
Set-VMNetworkAdapter -VMName "YourLinuxVM" -MacAddressSpoofing On
```
Luego, reinicie la máquina virtual.
