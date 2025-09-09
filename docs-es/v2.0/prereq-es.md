# Requisitos previos para la configuración del servidor

## Desactivar Hyper-Threading
Desactiva el Hyper-Threading en la BIOS/UEFI de tu sistema host. El Hyper-Threading también se conoce como Simultaneous Multi Threading (SMT) en sistemas AMD. Desactivarlo es muy importante para obtener un rendimiento óptimo del filtrado cpumap con XDP y, por ende, mejorar el rendimiento y la latencia.

- Inicia el sistema y presiona la tecla correspondiente para entrar a la configuración de la BIOS.
- En sistemas AMD, deberás navegar por las opciones hasta encontrar la opción llamada "SMT Control". Normalmente se encuentra en: ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Una vez localizada, cambia la opción a "Disabled" o "Off".
- En sistemas Intel, también deberás buscar la opción para desactivar el Hyper-Threading en ajustes. En servidores HP, por ejemplo, está ubicada en: ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options```
- Guarda los cambios y reinicia el sistema.

## Desactivar SR-IOV en la BIOS

SR-IOV puede desactivar XDP nativo (modo controlador) en las Funciones Físicas (PF), forzando XDP Genérico (SKB) y reduciendo el rendimiento y la estabilidad de LibreQoS. Desactive SR-IOV en la BIOS/UEFI para las tarjetas de red (NIC) utilizadas por LibreQoS. Si existen opciones por ranura/por puerto, desactívelas.
