# Requisitos del Sistema

LibreQoS puede ejecutarse en un servidor físico dedicado (bare metal) o como una máquina virtual (VM). El sistema operativo soportado es Ubuntu Server 24.04.

## Servidor Físico (Bare Metal)

### CPU
* Se requieren 2 o más núcleos de CPU.
* Eliga una CPU con alto [rendimiento de un solo hilo](https://www.cpubenchmark.net/singleThread.html#server-thread) dentro de su presupuesto. El regulamiento es intensivo en CPU y requiere alto rendimiento en un solo hilo.

El rendimiento de un solo hilo determina la capacidad máxima de un único HTB (núcleo de CPU). Esto, a su vez, afecta la capacidad máxima de cualquier nodo de nivel superior en la jerarquía de red (por ejemplo, sitios principales en su red). También impacta la velocidad máxima de plan que puede ofrecer a sus clientes dentro de márgenes seguros.

| Puntaje Single-Thread | Nodo Superior Máx. | Plan de Cliente Máx |
|:-------------------:|:------------------:|:-----------------:|
| 1000                | 1 Gbps             | 100 Mbps          |
| 1500                | 2 Gbps             | 500 Mbps          |
| 2000                | 3 Gbps             | 1 Gbps            |
| 3000                | 4 Gbps             | 2 Gbps            |
| 4000                | 5 Gbps             | 3 Gbps            |

A continuación se muestra una tabla de la capacidad agregada aproximada, suponiendo que una CPU tenga una puntuación de rendimiento de un   [solo hilo](https://www.cpubenchmark.net/singleThread.html#server-thread) de 1000, 2000, 3000 o 4000:

| Núcleos CPU | Puntaje de un solo hilo: 1000 | Puntaje de un solo hilo: 2000 | Puntaje de un solo hilo: 3000 | Puntaje de un solo hilo: 4000 |
|:---------:|:-------------------------:|:-------------------------:|:-------------------------:|:-------------------------:|
| 2         | 1 Gbps                    | 3 Gbps                    | 5 Gbps                    | 7 Gbps                    |
| 4         | 3 Gbps                    | 5 Gbps                    | 9 Gbps                    | 13 Gbps                   |
| 6         | 4 Gbps                    | 8 Gbps                    | 14 Gbps                   | 20 Gbps                   |
| 8         | 5 Gbps                    | 10 Gbps                   | 18 Gbps                   | 27 Gbps                   |
| 16        | 10 Gbps                   | 21 Gbps                   | 36 Gbps                   | 54 Gbps                   |
| 32        | 21 Gbps                   | 42 Gbps                   | 72 Gbps                   | 108 Gbps                  |
| 64        | 42 Gbps                   | 84 Gbps                   | 144 Gbps                  | 216 Gbps                  |
| 128       | 84 Gbps                   | 168 Gbps                  | 288 Gbps                  |                           |

### Hyper-threading

Se recomienda desactivar Hyper-Threading (Simultaneous Multi-Threading) en la configuración del BIOS/UEFI, ya que puede interferir con el procesamiento de XDP.

### Memory
* RAM Recomendada:

| RAM (usando CAKE) | Máx. Suscriptores |
| ---------------- | --------------- |
| 8 GB             | 1,000           |
| 16 GB            | 2,500           | 
| 32 GB            | 5,000           |
| 64 GB            | 10,000          |
| 128 GB           | 20,000          |

### Espacio en Disco

Se recomienda disponer de 50 GB o más de espacio en disco, tanto para servidores físicos como para implementaciones en máquinas virtuales.

### Recomendaciones de Dispositivos
#### Dispositivo de Espacio Pequeño (1G a 10G)

|        Rendimiento       |                                         10 Gbps                                        |
|:-----------------------:|:--------------------------------------------------------------------------------------:|
| Por Nodo / Por Núcleo | 5 Gbps                                                                                 |
| Fabricante            | Minisforum                                                                             |
| Modelo                   | [MS-01](https://store.minisforum.com/products/minisforum-ms-01?variant=46174128898293) |
| Opción de CPU             | i9-12900H                                                                              |
| Opción de RAM              | 1x32GB                                                                                 |
| Opción de NIC              | Built-in                                                                               |
| Rango de Temperatura              | 0°C ~ 40°C                                                                             |
| Rango de Temperatura              | (32°F ~ 104°F)                                                                         |
| ECC                     | No                                                                                     |
| Energía                   | 19V DC                                                                                 |

#### Servidores Rackmount (10G to 100G)

|        Rendimiento       |                                     10 Gbps                                    |                                                                                               10 Gbps                                                                                               |                                                  25 Gbps                                                 | 50 Gbps                                                                               | 100 Gbps                                                                            |
|:-----------------------:|:------------------------------------------------------------------------------:|:---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------:|:--------------------------------------------------------------------------------------------------------:|:---------------------------------------------------------------------------------------:|:-------------------------------------------------------------------------------------:|
| Por Nodo / Por Núcleo | 5 Gbps                                                                         | 5 Gbps                                                                                                                                                                                              | 3 Gbps                                                                                                   | 3 Gbps                                                                                | 4 Gbps                                                                              |
| Fabricante            | Supermicro                                                                     | Dell                                                                                                                                                                                                | Supermicro                                                                                               | Supermicro                                                                            | Supermicro                                                                          |
| Modelo                   | [SYS-511R-M](https://store.supermicro.com/us_en/mainstream-1u-sys-511r-m.html) | [PowerEdge R260](https://www.dell.com/en-us/shop/dell-poweredge-servers/new-poweredge-r260-rack-server/spd/poweredge-r260/pe_r260_tm_vi_vp_sb?configurationid=2cd33e43-57a3-4f82-aa72-9d5f45c9e24c) | [AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | [AS-1015SV-WTNRT](https://store.supermicro.com/us_en/1u-amd-wio-as-1015sv-wtnrt.html) | [AS -2015CS-TNR](https://store.supermicro.com/us_en/clouddc-amd-as-2015cs-tnr.html) |
| Opción de CPU              | E-2488                                                                         | E-2456                                                                                                                                                                                              | 8534P                                                                                                    | 8534P                                                                                 | 9745                                                                                |
| Opción de RAM              | 1x32GB                                                                         | 1x32GB                                                                                                                                                                                              | 4x16GB                                                                                                   | 2x64GB                                                                                | 4x64GB                                                                              |
| Opción de NIC              | 10-Gigabit X710-BM2 (2 x SFP+)                                                 | Intel X710-T2L (2 x 10G RJ45)                                                                                                                                                                       | Mellanox (2 x SFP28)                                                                                     | Mellanox 100-Gigabit (2 x QSFP56)                                                     | MCX653106A-HDAT                                                                     |
| Rango de Temperatura              | 0°C ~ 40°C                                                                     | 5–40°C                                                                                                                                                                                              | 0°C ~ 40°C                                                                                               | 0°C ~ 40°C                                                                            | 0°C ~ 40°C                                                                          |
| Rango de Temperatura              | (32°F ~ 104°F)                                                                 | (41–104°F)                                                                                                                                                                                          | (32°F ~ 104°F)                                                                                           | (32°F ~ 104°F)                                                                        | (32°F ~ 104°F)                                                                      |
| ECC                     | Sí                                                                            | Sí                                                                                                                                                                                                 | Sí                                                                                                      |                  Sí                                                                                   |                                  Sí                                                                                 |
| Energía                   | AC                                                                             | AC                                                                                                                                                                                                  | AC                                                                                                       | AC                                                                                    | AC                                                                                  |

Otra solución rentable es adquirir un servidor usado de un proveedor de confianza, como [TheServerStore](https://www.theserverstore.com/).
Estos proveedores suelen ofrecer servidores capaces de manejar un rendimiento de 10 Gbps, por alrededor de 500 USD.

### Requisitos de Interfaces de Red
* Una interfaz de red de administración completamente separada de las interfaces de regulamiento de tráfico. Usualmente esta sería la interfaz Ethernet integrada en la placa base.
* Una Tarjeta de Red Dedicada para Dos Interfaces de Regulamiento

Las Tarjetas de Red oficialmente soportadas para las dos interfaces de regulamiento se enumeran a continuación:

| Controlador NIC         | Velocidad de Puerto       | Modelos Sugeridos                                                                        | Problemas Conocidos                                                                                  |
|------------------------|------------------|-----------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| Intel X520             | 10 Gbps          |                                                                                         | Compatibilidad de módulos*                                                                         |
| Intel X710             | 10 Gbps          | [X710-BM2 10G]( https://www.fs.com/products/75600.html?now_cid=4253)                    | Compatibilidad de módulos*                                                                         |
| Intel XXV710           | 10 / 25 Gbps     | [XXV710 25G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896)         | Compatibilidad de módulos*                                                                         |
| Intel XL710            | 10 / 40 Gbps     | [XL710-BM2 40G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896 )     | Compatibilidad de módulos*                                                                         |
| Mellanox ConnectX-4 Lx | 10/25/40/50 Gbps |                                                                                         | No se conocen problemas.                                                                              |
| Mellanox ConnectX-6    | 10/25 Gbps       | [MCX631102AN-ADAT](https://www.fs.com/products/212177.html?now_cid=4014)                | No se conocen problemas.                                                                              |
| Mellanox ConnectX-6    | 100 Gbps         | [MCX623106AN-CDAT 100G](https://www.fs.com/products/119646.html?now_cid=4014)           | No se conocen problemas.                                                                              |
| Mellanox ConnectX-7    | 200 Gbps         | [MCX755106AS-HEAT 200G](https://www.fs.com/products/242589.html?now_cid=4014)           | No se conocen problemas.                                                                              |

(*) Intel suele bloquear por fabricante la compatibilidad de módulos SFP+. Verifique la compatibilidad del módulo antes de comprar. Mellanox no presenta este problema.

**ÚNICAMENTE brindaremos soporte para sistemas que utilicen una NIC de la lista anterior**.  
Algunas otras NICs *podrían* funcionar, pero no estarán oficialmente soportadas por LibreQoS.  
Si deseas *probar* la compatibilidad de otra tarjeta, ten en cuenta estos requisitos fundamentales de NIC:
  * La NIC debe tener múltiples filas de transmisión/recepción (TX/RX), mayores o iguales al número de núcleos de CPU. [Aquí se explica cómo verificarlo desde la terminal](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * La NIC debe tener [soporte de controlador XDP](https://github.com/xdp-project/xdp-project/blob/master/areas/drivers/README.org) para alto rendimiento (10 Gbps o más).

Si descubres que una tarjeta no listada en la tabla anterior es compatible, por favor háznoslo saber enviando un correo a support@libreqos.io.

## Máquina Virtual
LibreQoS puede ejecutarse como una máquina virtual (VM), aunque esto implica una penalización de rendimiento del 30%. Para VMs, se requiere "atravesamiento de NIC" para alcanzar un rendimiento superior a 10 Gbps (XDP vs XDP genérico).  
LibreQoS requiere 2 o más filas RX/TX, por lo que al usar una plataforma de virtualización como Proxmox, asegúrese de habilitar [Multiqueue](https://forum.proxmox.com/threads/where-is-multiqueue.146783/) para las interfaces de regulamiento asignadas a la VM. El valor de "Multiqueue" debe ser igual al número de núcleos vCPU asignados a la VM.

| Rendimiento | vCPU* |  RAM  |  Disco |
|:-------:|:-----:|:-----:|:-----:|
| 1 Gbps  | 2     | 8 GB  | 50 GB |
| 10 Gbps | 8     | 32 GB | 50 GB |

* Se asume un rendimiento de vCPU igual al de un único núcleo del Intel Xeon E-2456 con "hyper-threading" deshabilitado.
