## Requerimientos de Sistema
### Servidor físico
* LibreQoS requiere un servidor físico x86_64 de manera dedicada.
* Aunque es técnicamente posible correr en una  VM, no está oficialmente soportado, y conlleva una pérdida de rendimiento significativa del 30 % (incluso al usar la transferencia de NIC). En las máquinas virtuales, la transferencia de NIC es necesaria para un rendimiento superior a 1 Gbps (XDP frente a XDP genérico).

### CPU
* 2 o más cores de CPU 
* Una CPU con un rendimiento sólido [single-thread performance](https://www.cpubenchmark.net/singleThread.html#server-thread) que se ajuste a su presupuesto. El encolado consume muchos recursos de la CPU y requiere un alto rendimiento de un solo hilo.

El rendimiento de la CPU de un solo subproceso determinará la capacidad máxima de un solo HTB (núcleo de CPU) y, a su vez, la capacidad máxima de cualquier nodo de nivel superior en la jerarquía de la red (por ejemplo, los sitios de nivel superior de su red). Esto también afecta el plan de velocidad máxima que puede ofrecer a sus clientes dentro de márgenes seguros.

| Máx nodos nivel     | Puntuación de un hilo    |
| --------------------| ------------------------ |
| 1 Gbps              | 1000                     |
| 2 Gbps              | 1500                     |
| 3 Gbps              | 2000                     |
| 5 Gbps              | 4000                     |

| Plan máximo cliente | Puntuación de un hilo    |
| --------------------| ------------------------ |
| 100 Mbps            | 1000                     |
| 250 Mbps            | 1250                     |
| 500 Mbps            | 1500                     |
| 1 Gbps              | 1750                     |
| 2.5 Gbps            | 2000                     |
| 5 Gbps              | 4000                     |

A continuación se muestra una tabla de capacidad agregada aproximada, suponiendo una CPU con una [hilo único](https://www.cpubenchmark.net/singleThread.html#server-thread) desempeño de 1000 / 2000 / 4000:

| Núcleos CPU | Puntuación de un solo hilo = 1000 | Puntuación de un solo hilo = 2000 | Puntuación de un solo hilo = 4000 |
|-----------|----------------------------|----------------------------|----------------------------|
| 2         | 1 Gbps                     | 3 Gbps                     | 7 Gbps                     |
| 4         | 3 Gbps                     | 5 Gbps                     | 13 Gbps                    |
| 6         | 4 Gbps                     | 8 Gbps                     | 20 Gbps                    |
| 8         | 5 Gbps                     | 10 Gbps                    | 27 Gbps                    |
| 16        | 10 Gbps                    | 21 Gbps                    | 54 Gbps                    |
| 32        | 21 Gbps                    | 42 Gbps                    | 108 Gbps                   |
| 64        | 42 Gbps                    | 83 Gbps                    | 216 Gbps                   |

### Memoria
* RAM Recomendada:

| RAM (usando CAKE)| Máx Suscriptores|
| ---------------- | --------------- |
| 8 GB             | 1,000           |
| 16 GB            | 2,000           | 
| 32 GB            | 5,000           |
| 64 GB            | 10,000          |
| 128 GB           | 20,000          |
| 256 GB           | 40,000          |

### Recomendaciones de servidor
Aquí hay algunas opciones de servidor listas para usar y convenientes para considerar:

| **Rendimiento**              | 2.5 Gbps                                                                                                           | 10 Gbps                                                                                                             | 10 Gbps                                                                                   | 10 Gbps                                                                                                                                                                                                  | 25 Gbps                                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **Por Nodo / Por núcleo CPU** | 1 Gbps                                                                                                             | 3 Gbps                                                                                                              | 5 Gbps                                                                                    | 5 Gbps                                                                                                                                                                                                   | 3 Gbps                                                                                                              |
| **Modelo**                   | [Supermicro SYS-E102-13R-E](https://store.supermicro.com/us_en/compact-embedded-iot-i5-1350pe-sys-e102-13r-e.html) | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | [Supermicro SYS-511R-M](https://store.supermicro.com/us_en/mainstream-1u-sys-511r-m.html) | [Dell PowerEdge R260](https://www.dell.com/en-us/shop/dell-poweredge-servers/new-poweredge-r260-rack-server/spd/poweredge-r260/pe_r260_tm_vi_vp_sb?configurationid=2cd33e43-57a3-4f82-aa72-9d5f45c9e24c) | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) |
| **Opción CPU**              | Default                                                                                                            | 8124P                                                                                                               | E-2488                                                                                    | E-2456                                                                                                                                                                                                   | 8534P                                                                                                               |
| **Opción RAM**              | 2x8GB                                                                                                              | 2x16GB                                                                                                              | 2x32GB                                                                                    | 2x32GB                                                                                                                                                                                                   | 4x16GB                                                                                                              |
| **Opción NIC**              | Incorporado                                                                                                           | Mellanox (2 x SFP28)                                                                                                | 10-Gigabit X710-BM2 (2 x SFP+)                                                            | Intel X710-T2L (2 x 10G RJ45)                                                                                                                                                                            | Mellanox (2 x SFP28)                                                                                                |
| **Extras**                  | [USB-C RJ45](https://www.amazon.com/Anker-Ethernet-PowerExpand-Aluminum-Portable/dp/B08CK9X9Z8/)                   |                                                                                                                     |                                                                                           |                                                                                                                                                                                                          |                                                                                                                     |
| **Rango de Temperatura**              | 0°C ~ 40°C                                                                                                         | 0°C ~ 40°C                                                                                                          | 0°C ~ 40°C                                                                                | 5–40°C                                                                                                                                                                                                   | 0°C ~ 40°C                                                                                                          |
| **Rango de Temperatura**              | (32°F ~ 104°F)                                                                                                     | (32°F ~ 104°F)                                                                                                      | (32°F ~ 104°F)                                                                            | (41–104°F)                                                                                                                                                                                               | (32°F ~ 104°F)                                                                                                      |

### Requisitos de la interfaz de red
* Una interfaz de red de administración completamente independiente de las interfaces de modelado de tráfico. Normalmente, esta sería la interfaz Ethernet integrada en la placa base.
* Una tarjeta de interfaz de red dedicada para dos interfaces de modelado

A continuación se enumeran las tarjetas de interfaz de red compatibles oficialmente para las dos interfaces de modelado.:

| Controlador NIC         | Velocidad Puerto       | Modelos  Sugeridos                                                                        | Problemas conocidos                                                                                  |
|------------------------|------------------|-----------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| Intel X520             | 10 Gbps          |                                                                                         | Compatibilidad de módulos*                                                                         |
| Intel X710             | 10 Gbps          | [X710-BM2 10G]( https://www.fs.com/products/75600.html?now_cid=4253)                    | Compatibilidad de módulos*                                                                         |
| Intel XXV710           | 10 / 25 Gbps     | [XXV710 25G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896)         | Módulo de compatibilidad*                                                                         |
| Intel XL710            | 10 / 40 Gbps     | [XL710-BM2 40G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896 )     | Compatibilidad de módulos*                                                                         |
| Mellanox ConnectX-4 Lx | 10/25/40/50 Gbps |                                                                                         | No hay problemas conocidos.                                                                              |
| Mellanox ConnectX-5    | 100 Gbps         | [MCX516A-CCAT 100G](https://www.fs.com/products/119647.html?attribute=67743&id=3746410) | Calor extremo con carga alta (más de 50 Gbps). Utilice el kit de refrigeración líquida para CPU en el chip para evitar el sobrecalentamiento. |
| Mellanox ConnectX-6    | 10/25 Gbps       | [MCX631102AN-ADAT](https://www.fs.com/products/212177.html?now_cid=4014)                | No hay problemas conocidos.                                                                              |
| Mellanox ConnectX-6    | 100 Gbps         | [MCX623106AN-CDAT 100G](https://www.fs.com/products/119646.html?now_cid=4014)           | No hay problemas conocidos.                                                                              |
| Mellanox ConnectX-7    | 200 Gbps         | [MCX755106AS-HEAT 200G](https://www.fs.com/products/242589.html?now_cid=4014)           | No hay problemas conocidos.                                                                              |

(*) Intel suele restringir la compatibilidad de los módulos SFP+. Verifique la compatibilidad del módulo antes de comprarlo. Mellanox no presenta este problema.

**SÓLO brindaremos soporte para sistemas que utilicen una NIC mencionada anteriormente**. Algunas otras tarjetas de red *podrían* funcionar, pero no serán compatibles oficialmente con LibreQoS. Si desea *probar* la compatibilidad de otra tarjeta, tenga en cuenta estos requisitos fundamentales de la tarjeta de red:
  * La NIC debe tener varias colas de transmisión TX/RX, mayores o iguales que la cantidad de núcleos de CPU. [Aquí se explica cómo comprobarlo desde la línea de comandos](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * NIC debe tener [compatibilidad controlador XDP](https://github.com/xdp-project/xdp-project/blob/master/areas/drivers/README.org) para alto rendimiento (10 Gbps+).

Si usted descubre que una tarjeta que no está listada en la tabla de arriba es compatible, por favor, háganoslo saber enviando un correo electrónico a support [at] libreqos.io. 
