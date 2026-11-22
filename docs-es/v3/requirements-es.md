# Requisitos del sistema

**Versión: LibreQoS v3.** Qué necesita un host v3 y cómo prepararlo.
`libreqos-v3-preflight` comprueba la mayor parte de esto; consulte
[Instalar LibreQoS v3 (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/install.html).

LibreQoS v3 funciona en un servidor físico dedicado o en una VM.

## Sistema operativo

- Ubuntu Server 24.04 LTS o posterior (preflight bloquea versiones anteriores).
  Consulte [Instalar Ubuntu Server](ubuntu-server-es.md).
- Acceso root o sudo.
- Unos 50 GB de disco.

## CPU

El bridge DPDK ejecuta un worker por núcleo de CPU, y cada worker sondea sus
colas de la NIC sin pausa. La configuración reserva dos núcleos físicos para el
sistema operativo, `lqosd` y los hilos de control del bridge, y usa el resto
para workers:

`workers = min(núcleos físicos − 2, colas de la NIC, objetivo según la velocidad del enlace)`

- **Más núcleos físicos significan más workers** y más capacidad. La
  configuración cuenta solo núcleos físicos; los hilos de Hyper-Threading (SMT)
  no se usan como workers.
- **El rendimiento por hilo importa.** Cada worker procesa los circuitos que
  tiene asignados, así que un núcleo más rápido mueve más tráfico por worker.
- **Desactive Hyper-Threading / SMT** en la BIOS/UEFI (recomendado). Un worker
  sondea sin pausa, y un hilo hermano ocupado en el mismo núcleo lo ralentiza.
  - AMD: normalmente `Advanced → AMD CBS → CPU Common Options → Thread
    Enablement → SMT Control`, en `Disabled`.
  - Intel: la opción "Hyper-Threading", por ejemplo en servidores HP en
    `System Configuration → BIOS/Platform Configuration (RBSU) → Processor
    Options`.

El rendimiento depende tanto del número de circuitos activos como del hardware;
indique siempre ambos juntos.

## Memoria

- El bridge necesita **3072 hugepages de 2 MiB (6 GiB)**. El instalador las
  reserva; si no puede, agregue `default_hugepagesz=2M hugepages=3072` a la línea
  de comandos del kernel y reinicie. Preflight bloquea cuando las páginas no
  están reservadas ni hay memoria libre para reservarlas.
- Sume la memoria del sistema operativo, `lqosd` y la WebUI a las hugepages.

## Interfaces de red

- **Una interfaz de administración**, separada del dataplane, normalmente el
  puerto Ethernet de la placa. La WebUI y SSH la usan.
- **Dos puertos de dataplane** para un equipo en línea, o uno para un trunk
  on-a-stick. La configuración los entrega a DPDK. No deben tener dirección IP
  ni un bridge, bond o VLAN encima.
- **No se admiten interfaces bond en el dataplane.**
- **AF_XDP no está soportado.** v3 usa DPDK en las NIC siguientes.

Familias de NIC de dataplane soportadas (los drivers que acepta preflight):

| Driver | Familia de NIC | Ejemplos |
|---|---|---|
| `mlx5_core` | NVIDIA/Mellanox ConnectX-4 Lx y posteriores | ConnectX-4 Lx, ConnectX-5, ConnectX-6, ConnectX-7 |
| `i40e` | Intel serie 700 | X710, XL710, XXV710 |
| `ice` | Intel serie 800 | E810 |
| `ixgbe` | Intel 10G | X520, X540, X550 |
| `igb` | Intel 1G | I350 |

Intel suele bloquear los módulos SFP+ por proveedor; compruebe la
compatibilidad de los módulos antes de comprar. Las tarjetas Mellanox no tienen
este problema.

La NIC necesita al menos tantas colas RX/TX como workers quiera; una NIC con
menos colas limita el número de workers.

Otras NIC pueden funcionar, pero no están soportadas. Si prueba una con éxito,
avísenos en support [at] libreqos.io.

## Máquinas virtuales

- Use la NIC virtio moderna (`1af4:1041`) con `disable-legacy=on` en ambas NIC
  del dataplane, o pase una NIC soportada a la VM con PCI passthrough.
- Asigne a la VM suficientes vCPU para los workers más los dos núcleos
  reservados, y memoria para las hugepages.
- **Proxmox:** habilite [Multiqueue](https://forum.proxmox.com/threads/where-is-multiqueue.146783/)
  en las NIC del dataplane, con el mismo número que de vCPU.

## Capacidad medida

Un host de referencia, medido en el laboratorio de LibreQoS con un generador de
tráfico TRex. Son ejecuciones únicas de 30 segundos por velocidad, no una
garantía; úselas como punto de partida y deje margen.

**Host:** AMD Ryzen 9 7950X (16 núcleos), 14 workers, NIC de dataplane NVIDIA
ConnectX (`mlx5`), el perfil de energía predeterminado `balanced` (turbo
desactivado, 4,5 GHz), 1.000 circuitos, CAKE en todos los circuitos.

**Tráfico similar al de producción.** Los tamaños de paquete y la mezcla de
subida/bajada provienen de la red de producción de un ISP: tramas de bajada de
1.291 bytes, tramas de subida de 386 bytes y unos 0,4 paquetes de subida por
cada paquete de bajada.

| Bajada ofrecida | Total entregado (ambos sentidos) | Paquetes/s (total) | Pérdida de bajada | Pérdida de subida |
|---:|---:|---:|---:|---:|
| 25 Gbps | 28,0 Gbps | 3,4 M | 0,007 % | 0,001 % |
| 40 Gbps | 44,8 Gbps | 5,4 M | 0,03 % | < 0,001 % |
| 50 Gbps¹ | 53,4 Gbps | 6,8 M | 4,9 % | 2,6 % |

¹ Ejecución de 15 segundos. A esta velocidad el host supera su capacidad.

Así, este host transporta unos **45 Gbps de tráfico similar al de producción con
pérdida despreciable**, unos **3,2 Gbps por worker**.

**El límite lo marca la tasa de paquetes, no la de bits.** Este host funciona
sin pérdidas a 5,4 millones de paquetes por segundo y supera su capacidad a 6,8
millones. Si su paquete medio es más pequeño que esta mezcla, espere menos Gbps:
estime su tasa de paquetes en hora pico (bits por segundo totales ÷ 8 ÷ tamaño
medio de paquete) y compárela.

**El perfil de energía importa.** Con el perfil `eco` (frecuencia limitada al
65 %), el mismo host perdió el 16,6 % del tráfico de bajada a 40 Gbps, frente
al 0,03 % de `balanced`. Consulte [Perfil de energía de la CPU](power-es.md).

## Hardware de ejemplo

Estos servidores usan NIC de la lista soportada. No se ha medido el rendimiento
v3 por modelo; las cifras anteriores son solo del host de referencia.

| Clase | Modelo | CPU | NIC |
|---|---|---|---|
| Formato pequeño | Minisforum MS-01 | i9-12900H | Intel X710 integrada (2 × SFP+) |
| 1U | Supermicro SYS-511R-M | Xeon E-2488 | Intel X710-BM2 (2 × SFP+) |
| 1U | Dell PowerEdge R260 | Xeon E-2456 | Intel X710-T2L (2 × 10G RJ45) |
| 1U | Supermicro AS-1115S-FWTRT | EPYC 8534P | Mellanox (2 × SFP28) |
| 1U | Supermicro AS-1015SV-WTNRT | EPYC 8534P | Mellanox (2 × QSFP56) |
| 2U | Supermicro AS-2015CS-TNR | EPYC 9745 | Mellanox MCX653106A-HDAT |

En el MS-01, cambiar la pasta térmica reduce notablemente la temperatura de la
CPU (puede anular la garantía). Un servidor usado de un proveedor confiable
también es una opción económica.
