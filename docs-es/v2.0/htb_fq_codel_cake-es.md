# HTB + fq_codel + CAKE: Comportamiento Detallado de Colas en LibreQoS

Esta página es el complemento de análisis profundo de colas para [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md).

Explica:

1. Por qué LibreQoS combina `HTB` con qdiscs hoja (`fq_codel` o `CAKE`)
2. Cómo funciona `fq_codel` en la práctica
3. Cómo funciona `CAKE` en la práctica
4. Cuándo elegir `fq_codel` vs `CAKE`
5. Patrones de observabilidad y troubleshooting para operadores

## 1) Por Qué Existen Estos Tres Componentes Juntos

En LibreQoS en producción:

1. `HTB` entrega política jerárquica de tasa (`rate`, `ceil`, préstamo, jerarquía)
2. `fq_codel` o `CAKE` entrega servicio de cola por flujo y AQM dentro de cada envolvente moldeada

Esta separación es intencional:

- Problema de política: "¿Cuánto puede enviar esta clase?" -> `HTB`
- Problema de cola: "¿Qué paquete sale después mientras se controla latencia?" -> `fq_codel`/`CAKE`

## 2) Ubicación en Tiempo de Ejecución en LibreQoS

Conceptualmente, los paquetes pasan por:

1. raíz `mq`
2. jerarquía `HTB`
3. qdisc hoja por clase moldeada (`CAKE` por defecto, `fq_codel` opcional)

Operativamente, suele ser:

`mq` raíz -> padres HTB por CPU -> clases HTB por circuito -> qdisc hoja (`cake diffserv4` o `fq_codel`)

Cada clase HTB tiene un punto de acople para qdisc hijo. Si no hay qdisc hoja explícito, se usa el comportamiento de cola por defecto del kernel para esa clase.

Modelo práctico de comportamiento en LibreQoS:

1. La política por defecto de fábrica usa `HTB` + `cake diffserv4` para circuitos moldeados.
2. TreeGuard (función próxima) puede cambiar dinámicamente direcciones de circuito entre `cake diffserv4` y `fq_codel` según guardrails de baja carga/RTT.
3. TreeGuard no está habilitado por defecto.

Consulte [TreeGuard (Función Próxima de v2.0)](treeguard-es.md) para detalles de configuración y despliegue.

## 3) Resumen de HTB para Usuarios de AQM

### 3.1 Mecánicas centrales de HTB

1. Los tokens se consumen por bytes de paquete y se recargan con el tiempo.
2. `rate` define servicio garantizado.
3. `ceil` define el máximo prestable cuando existe capacidad en el padre.
4. Los hijos solo piden prestado de capacidad sobrante en ancestros.
5. La contención entre hermanos depende de `prio`, del scheduler y de la proporción de clases.

### 3.2 Controles clave de HTB

- `rate`, `ceil`
- `prio`
- `burst`, `cburst`
- `quantum`, `r2q`
- clase `default` (concepto Linux HTB; ver nota de comportamiento de LibreQoS)

### 3.3 Por qué importa para `fq_codel` y `CAKE`

`fq_codel` y `CAKE` no reemplazan jerarquía ni política de tasa de HTB. Gestionan servicio de cola dentro de la envolvente que HTB permite.

### 3.4 Comportamiento de tráfico no definido en LibreQoS

El comportamiento de LibreQoS es explícito:

1. el tráfico no mapeado a un circuito moldeado pasa de largo (pass-through)
2. LibreQoS no dirige tráfico no definido a clases HTB `default`
3. el comportamiento de clase `default` de HTB sigue existiendo en Linux `tc`, pero no es la vía usada por LibreQoS para tráfico no definido

Operativamente, esto implica que el troubleshooting de tráfico no definido comienza con validación de clasificación/mapeo, no con ajuste de clase `default`.

### 3.5 Esqueleto compacto HTB (patrón de referencia)

Patrón ilustrativo Linux HTB + qdisc hoja:

```bash
tc qdisc add dev <ifname> root handle 1: htb default 30
tc class add dev <ifname> parent 1: classid 1:1 htb rate 1gbit ceil 1gbit
tc class add dev <ifname> parent 1:1 classid 1:10 htb rate 700mbit ceil 1gbit prio 1
tc class add dev <ifname> parent 1:1 classid 1:20 htb rate 300mbit ceil 1gbit prio 2
tc class add dev <ifname> parent 1:1 classid 1:30 htb rate 10mbit ceil 1gbit prio 7
tc qdisc add dev <ifname> parent 1:10 cake diffserv4
tc qdisc add dev <ifname> parent 1:20 fq_codel
tc qdisc add dev <ifname> parent 1:30 cake diffserv4
tc filter add dev <ifname> protocol ip parent 1:0 prio 1 u32 match ip src <A>/32 flowid 1:10
tc filter add dev <ifname> protocol ip parent 1:0 prio 2 u32 match ip src <B>/32 flowid 1:20
```

En LibreQoS, los comandos de colas/clases se generan automáticamente y el tráfico no definido pasa de largo en lugar de enviarse a una clase HTB `default`.

## 4) Profundización en fq_codel

### 4.1 Qué es fq_codel

`fq_codel` combina:

1. colas estocásticas por flujo (hash)
2. planificación de equidad estilo DRR entre colas
3. AQM CoDel por cola

Referencias principales:

- `tc-fq_codel(8)`
- RFC 8290 (Flow Queue CoDel)

### 4.2 Comportamiento del scheduler y ventaja para flujos esporádicos

FQ-CoDel mantiene listas activas de flujos "new" y "old". Las colas recién activadas se priorizan frente a colas persistentemente en backlog, lo que beneficia tráfico interactivo/esporádico.

También utiliza crédito por bytes (`quantum`), por lo que la equidad es por bytes y no por cantidad de paquetes.

### 4.3 Modelo de hash de flujos

Por defecto, los paquetes se clasifican con hash de 5-tupla hacia un número configurable de buckets (`flows`). Colisiones de hash son posibles y forman parte del compromiso de las colas estocásticas.

### 4.4 Parámetros de fq_codel que realmente se ajustan

Parámetros útiles de `tc-fq_codel(8)`:

1. `limit PACKETS`: tope duro de paquetes en cola (por defecto `10240`)
2. `memory_limit BYTES`: tope de memoria (por defecto `32MB`); se aplica el menor entre `limit` y memoria
3. `flows NUMBER`: buckets hash (por defecto `1024`, se define al crear)
4. `target TIME`: retardo mínimo persistente aceptable (por defecto `5ms`)
5. `interval TIME`: ventana de control CoDel; normalmente del orden del RTT peor en el cuello de botella (por defecto `100ms`)
6. `quantum BYTES`: quantum DRR (por defecto `1514`)
7. `ecn`/`noecn`: ECN encendido/apagado (`ecn` por defecto en fq_codel)
8. `ce_threshold TIME`: umbral de marcado ECN bajo para casos tipo DCTCP
9. `ce_threshold_selector VALUE/MASK`: aplica CE threshold solo al tráfico seleccionado
10. `drop_batch`: máximo lote de drops cuando se exceden límites (por defecto `64`)

### 4.5 Observabilidad fq_codel (`tc -s qdisc show`)

Campos comunes a revisar:

1. `dropped`, `overlimits`, `requeues`
2. `drop_overlimit`
3. `new_flow_count`
4. `ecn_mark`
5. `new_flows_len`, `old_flows_len`
6. `backlog`

Patrón de interpretación:

1. Verifique que exista presión de cola (`backlog`, `requeues`, `overlimits`)
2. Verifique si AQM está actuando (`ecn_mark`, `dropped`)
3. Correlacione `new_flows_len`/`old_flows_len` con mezcla de tráfico (esporádico vs masivo)

## 5) Profundización en CAKE

### 5.1 Arquitectura de CAKE

CAKE integra varias capas en un único qdisc:

1. shaper en modo deficit
2. cola de prioridad (tins)
3. aislamiento de flujo (`DRR++`)
4. AQM (`COBALT`, combina CoDel + BLUE)
5. gestión de paquetes y compensación de overhead

Referencias principales:

- `tc-cake(8)`
- páginas CAKE y CakeTechnical de Bufferbloat
- paper Piece of CAKE (`cake.pdf`)

### 5.2 Operación con shaping vs sin shaping

Cuando se define `bandwidth`, el shaper de CAKE y su ajuste derivado gobiernan umbrales de tins y comportamiento temporal.

Sin shaping (`unlimited`), CAKE aún aporta servicio de cola y lógica AQM, pero el servicio de tins ya no opera contra un objetivo fijo de cuello de botella moldeado.

### 5.3 Modos de aislamiento de flujo

CAKE soporta múltiples modos de equidad:

1. `flowblind` (sin aislamiento por flujo)
2. `flows` (equidad por flujo de 5-tupla)
3. `srchost`, `dsthost`, `hosts`
4. `dual-srchost`, `dual-dsthost`
5. `triple-isolate` (valor por defecto en `tc-cake(8)`)

Nota operativa:

- `triple-isolate` es un valor seguro cuando se requiere control tanto por flujo como por host.

### 5.4 Conciencia de NAT

`nat`/`nonat` controla si CAKE hace lookup de NAT antes de aplicar aislamiento de flujo.

Por qué importa:

- Sin `nat`, la equidad ve solo direcciones post-NAT.
- Con `nat`, la equidad puede representar mejor hosts internos detrás de NAT (si NAT está en la misma ruta/caja).

### 5.5 Modos DiffServ y tins

Presets principales de prioridad:

1. `besteffort` (un solo tin, sin cola de prioridad)
2. `diffserv3`
3. `diffserv4`
4. `diffserv8`
5. `precedence` (legado, desaconsejado en despliegues modernos)

`tc-cake(8)` documenta actualmente `diffserv3` como default general, mientras que LibreQoS típicamente usa `cake diffserv4` como política por defecto de fábrica para operadores.

### 5.6 Mapeo DSCP `diffserv4` en LibreQoS

LibreQoS usa comúnmente CAKE con `diffserv4`. Mapeo práctico de clases:

1. Sensible a latencia: `CS7`, `CS6`, `EF`, `VA`, `CS5`, `CS4`
2. Streaming multimedia: `AF4x`, `AF3x`, `CS3`, `AF2x`, `TOS4`, `CS2`, `TOS1`
3. Best Effort: `CS0`, `AF1x`, `TOS2` y codepoints no reconocidos
4. Tráfico de fondo: `CS1`

Codepoints comunes en uso operativo:

1. `CS1` (Least Effort)
2. `CS0` (Best Effort)
3. `TOS1` (Max Reliability / LLT "Lo")
4. `TOS2` (Max Throughput)
5. `TOS4` (Min Delay)
6. `TOS5` (LLT "La")
7. `AF1x`
8. `AF2x`
9. `AF3x`
10. `AF4x`
11. `CS2`
12. `CS3`
13. `CS4`
14. `CS5`
15. `CS6`
16. `CS7`
17. `VA`
18. `EF`

Marco de clases de tráfico estilo RFC 4594 (alto nivel):

1. Control de red: `CS6`, `CS7`
2. Telefonía: `EF`, `VA`
3. Señalización: `CS5`
4. Videoconferencia multimedia: `AF4x`
5. Interactivo en tiempo real: `CS4`
6. Streaming multimedia: `AF3x`
7. Video broadcast: `CS3`
8. Datos de baja latencia: `AF2x`, `TOS4`
9. Operaciones/administración/gestión: `CS2`, `TOS1`
10. Servicio estándar: `CS0` y codepoints no reconocidos
11. Datos de alto throughput: `AF1x`, `TOS2`
12. Datos de baja prioridad: `CS1`

Nota para `fq_codel`:

1. `fq_codel` no tiene modelo de tins de CAKE ni scheduler de clases DSCP estilo CAKE.
2. El marcado DSCP puede usarse por políticas externas de clasificación, pero no vía comportamiento de tins `diffserv4` de CAKE.
3. En LibreQoS, la prioridad DSCP descrita arriba aplica cuando se selecciona CAKE con `diffserv4`.

### 5.7 Compensación de overhead y framing

CAKE puede contabilizar overhead/framing de capa de enlace usando:

1. `overhead N`
2. `mpu N`
3. `atm`, `ptm`, `noatm`
4. atajos (`ethernet`, `docsis`, etc.)
5. `raw` y `conservative`

Regla operativa:

- Si overhead/framing está mal, el shaping también estará mal. Valide con pruebas de tráfico realistas.

### 5.8 Manejo de GSO (`split-gso`)

Por defecto, CAKE divide superpaquetes GSO para reducir impacto de latencia en flujos competidores, especialmente a tasas bajas.

En velocidades muy altas (ej. >10 Gbps), `no-split-gso` puede mejorar throughput pico, con posible costo en suavidad de latencia.

### 5.9 Filtrado ACK

CAKE soporta:

1. `ack-filter`
2. `ack-filter-aggressive`
3. `no-ack-filter` (por defecto)

El mejor caso de uso es enlace asimétrico donde ACKs en subida limitan el goodput de bajada. Aplíquelo con cautela y valide con tráfico real de aplicaciones, no solo pruebas sintéticas.

### 5.10 Modo ingress y autorate

`ingress` cambia contabilidad y ajuste para realidades de shaping en bajada (incluye contar paquetes descartados como datos ya transitados).

`autorate-ingress` puede estimar capacidad desde el tráfico entrante y es útil principalmente en enlaces muy variables (por ejemplo algunos enlaces celulares). No puede estimar cuellos de botella que estén aguas abajo del punto donde se adjunta CAKE.

### 5.11 Observabilidad CAKE (`tc -s qdisc show`)

Campos útiles frecuentes:

1. nivel superior: `dropped`, `overlimits`, `backlog`, `memory used`, `capacity estimate`
2. por tin: `thresh`, `target`, `interval`
3. telemetría de delay: `pk_delay`, `av_delay`, `sp_delay`
4. hashing: `way_inds`, `way_miss`, `way_cols`
5. señalización: `drops`, `marks`
6. filtrado ACK: `ack_drop`
7. actividad de cola: `sp_flows`, `bk_flows`, `un_flows`, `max_len`, `quantum`

Patrón de interpretación:

1. confirme que tins y umbrales coinciden con la política esperada
2. inspeccione EWMAs de delay por tin
3. correlacione `drops`/`marks` con latencia y throughput observados
4. monitoree indicadores de colisión hash (`way_cols`) bajo concurrencia alta

## 6) Marco de Política en LibreQoS: CAKE vs fq_codel

Para operadores LibreQoS, parta del comportamiento de plataforma:

1. La operación por defecto es `cake diffserv4` en hojas de clase HTB.
2. TreeGuard (función próxima) puede mover direcciones seleccionadas de circuito a `fq_codel` durante baja carga sostenida y volver a `cake` cuando sube utilización o presión de guardrails.
3. Los overrides manuales por circuito con `sqm` siguen dando control explícito al operador.

Use esta matriz para contexto de tradeoffs:

| Dimensión | `fq_codel` | `CAKE` |
|---|---|---|
| Complejidad de configuración | Menor | Mayor (más funciones integradas) |
| Huella de recursos a escala | Usualmente menor | Usualmente mayor |
| Funciones de shaping integradas | No (requiere shaper padre como HTB) | Sí (shaper deficit-mode integrado) |
| Comportamiento DiffServ/tins | Básico/indirecto | Modelo de tins nativo robusto |
| Modos de aislamiento de host | No estilo CAKE | Modos ricos de aislamiento host/flujo |
| Compensación de overhead | Limitada | Controles ricos de framing/overhead |
| Optimización ACK en enlaces asimétricos | No | Sí (modos de ACK filtering) |
| Mejor encaje | Muchas colas con recursos ajustados | Tráfico mixto con prioridad en riqueza de política y suavidad |

## 7) Notas Operativas de LibreQoS

Desde pruebas de mantenedor y feedback de despliegue:

1. `fq_codel` no tiene rate limiting intrínseco; depende de HTB para política de tasa.
2. `fq_codel` y `CAKE` mantienen tablas de estado por flujo, por lo que RAM/hash importan a gran escala.
3. `CAKE` y HTB son viables incluso en enlaces de tasa muy baja y asimétricos.
4. Un patrón de limitador tipo "sandwich" con HTB+fq_codel puede ser práctico en algunos entornos.
5. Algunas vistas de tráfico de dashboard reflejan contexto pre-drop; interprete contadores junto con semántica de drop/mark.
6. La frase "solo la saturación dura se beneficia de AQM" es demasiado estrecha; AQM y fair queueing pueden mejorar latencia bajo presión de cola (bursts/contención), incluso antes de que la interfaz esté al 100%.
7. Patrones históricos de cubeta compartida/default refuerzan que la dinámica de colas, y no solo "enlace completamente pegado", impulsa el valor de AQM; en LibreQoS actual, tráfico no definido es pass-through, así que aplique este principio a hojas HTB gestionadas.

## 8) Flujo Práctico de Observabilidad

Comience con:

```bash
tc -s qdisc show dev <ifname>
tc -s class show dev <ifname>
```

Luego:

1. confirme que existan clases HTB donde espera
2. confirme tipo de qdisc hoja (`cake` vs `fq_codel`) por clase
3. inspeccione contadores de clase y qdisc en conjunto
4. verifique que la dirección (`ingress`/`egress`) corresponda al problema
5. correlacione con latencia/throughput visibles al usuario, no solo con contadores

## 9) Malentendidos Comunes

1. "`fq_codel` o `CAKE` reemplaza HTB"
   - Falso para la jerarquía de LibreQoS; HTB sigue siendo la envolvente de política.
2. "El tráfico no definido va a una cola HTB default en LibreQoS"
   - Falso; LibreQoS deja pasar el tráfico no definido.
3. "Solo eventos de saturación dura se benefician de AQM"
   - Falso. Vea [Por qué AQM sigue ayudando incluso por debajo de la saturación de la tasa de línea](libreqos-backend-architecture-es.md#por-que-aqm-sigue-ayudando-incluso-por-debajo-de-la-saturacion-de-la-tasa-de-linea).
4. "Ajustar qdisc hoja arregla jerarquía/mapeo roto"
   - Falso; primero hay que corregir errores de mapeo/jerarquía.
5. "`fq_codel` puede limitar velocidad por sí solo"
   - Falso; use HTB (u otro shaper) para política explícita de tasa.

## 10) Contexto HTB HOWTO (Histórico, Aún Útil)

Material clásico del HTB HOWTO sigue siendo útil como modelo mental, traducido al LibreQoS moderno:

1. clasificar tráfico
2. programar servicio de colas
3. moldear en el cuello de botella (o justo aguas arriba)
4. definir intención explícita de clase con `rate`/`ceil`

Notas de traducción moderna:

1. confirme comportamiento con contadores `tc -s`, no con suposiciones de defaults
2. mantenga orden de clasificadores de forma intencional (reglas específicas antes de amplias)
3. incluya manejo catch-all explícito en despliegues manuales `tc`
4. en LibreQoS específicamente, tráfico no definido es pass-through salvo mapeo explícito a jerarquía moldeada

## 11) Referencias

- [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md)
- [TreeGuard (Función Próxima de v2.0)](treeguard-es.md)
- [tc-htb man page (man7)](https://man7.org/linux/man-pages/man8/tc-htb.8.html)
- [tc-fq_codel man page (man7)](https://man7.org/linux/man-pages/man8/tc-fq_codel.8.html)
- [tc-cake man page (man7)](https://man7.org/linux/man-pages/man8/tc-cake.8.html)
- [FlowQueue-Codel RFC 8290](https://www.rfc-editor.org/rfc/rfc8290)
- [IANA DSCP Registry](https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml)
- [CAKE wiki (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/Cake/)
- [CAKE technical notes (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/CakeTechnical/)
- [FQ_Codel wiki (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/FQ_Codel/)
- [CAKE vs FQ_Codel (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/Cake_vs_FQ_CODEL/)
- [CoDel/fq_codel wiki index (Bufferbloat)](https://www.bufferbloat.net/projects/codel/wiki/)
- [Toke Hoiland-Jorgensen, Dave Taht, Jonathan Morton. *Piece of CAKE: A Comprehensive Queue Management Solution for Home Gateways*, IEEE LANMAN, 2018.](https://arxiv.org/abs/1804.07617)
