# Arquitectura Backend y Diseño de Colas de LibreQoS

Esta página explica cómo se integran los sistemas backend de LibreQoS en tiempo de ejecución:

1. Tratamiento de paquetes en el plano de datos (`XDP`/`eBPF` -> `tc` -> árbol de colas)
2. Diseño de jerarquía de colas (`mq` + `HTB` + qdiscs hoja)
3. Comportamiento de AQM (`fq_codel`, `CAKE`) y por qué puede haber beneficios por debajo de la tasa máxima
4. Actualizaciones del plano de control (Scheduler, `lqosd`, Bakery, incremental vs recarga completa)
5. Límites de diseño prácticos para operadores

## Contexto de Fuente

Esta página incorpora detalles de publicaciones públicas del devblog:

- [Introducing the LibreQoS Bakery](https://devblog.libreqos.com/posts/0005-lqos-bakery/)
- [Fixing the Reload Penalty in LibreQoS](https://devblog.libreqos.com/posts/0013-no-more-locks/)

## 1) Modelo Mental del Backend

LibreQoS tiene dos planos que cooperan:

1. Plano de datos:
   - clasificar paquetes rápidamente
   - mapear paquetes a clases de cola
   - aplicar comportamiento de equidad y latencia a velocidad de línea
2. Plano de control:
   - calcular el estado deseado desde `network.json` y `ShapedDevices.csv`
   - aplicar el conjunto más pequeño de cambios de cola que sea seguro
   - evitar recargas innecesarias

En términos operativos:

- `XDP`/`eBPF` y mapas de lookup determinan identidad de paquete y ruta de CPU.
- Linux Traffic Control (`tc`) aplica la política de colas.
- Bakery gestiona deltas de actualización y límites de recarga.

## 2) Invariantes de Ejecución

Estos invariantes ayudan a evaluar si el comportamiento backend es saludable.

| Invariante | Por qué importa | Síntoma cuando se rompe | Primera verificación |
|---|---|---|---|
| Cada circuito moldeado mapea a un padre válido en la jerarquía | Sin padre no hay ubicación efectiva de cola | Suscriptores sin shaping o fuera de límites esperados | Validar relaciones de padre en `network.json` y en la entrada de dispositivos |
| Los supuestos de raíz multi-cola coinciden con NIC/entorno real | La distribución en CPU depende del modelo de colas | Un núcleo saturado y otros ociosos, shaping inestable bajo carga | Verificar modelo de colas de NIC y layout `mq`/clases con salida de `tc` |
| El mapeo del plano de datos es estable entre XDP y `tc` | Mapeo erróneo causa asignación de cola incorrecta | Contadores de clase inesperados, tráfico mal contabilizado | Comparar class IDs esperados vs contadores `tc -s` observados |
| Cambios del plano de control quedan dentro de límites incrementales seguros | Reduce disrupción por reconstrucciones completas | Ventanas frecuentes de recarga con impacto en paquetes | Revisar patrón de cambios: estructurales vs velocidad/mapeo |
| El número de colas cabe en presupuesto de CPU/RAM | Escala de qdisc hoja impacta recursos directamente | Crecimiento de memoria, recargas lentas, jitter bajo churn | Monitorear cantidad de colas, margen de RAM y cadencia de cambios |

## 3) Ruta End-to-End de Paquetes y Control

```{mermaid}
flowchart LR
    subgraph DP[Plano de Datos]
      A[Paquete de Ingreso] --> B[Parseo XDP: VLAN/PPPoE/IP/puertos]
      B --> C[Mapeo por cache de flujo y/o LPM]
      C --> D[Asignacion de CPU via cpumap]
      D --> E[Traspaso de metadata a clasificador tc]
      E --> F[Seleccion de clase tc]
      F --> G[Raiz mq]
      G --> H[Jerarquia HTB]
      H --> I[qdisc hoja: CAKE o fq_codel]
      I --> J[Egreso]
    end

    subgraph CP[Plano de Control]
      K[Entradas del Scheduler\nnetwork.json + ShapedDevices.csv] --> L[Estado deseado + buffer de comandos]
      L --> M[Bus de comandos lqosd]
      M --> N[Motor de diffs Bakery]
      N --> O[Actualizaciones tc incrementales]
      N --> P[Recarga completa controlada]
    end

    N -. actualiza estado de colas .-> H
```

## 4) Diseño del Plano de Datos

### 4.1 Pipeline XDP y clasificación

LibreQoS realiza trabajo temprano de paquetes en XDP cuando es posible:

1. Parsear encabezados una sola vez.
2. Resolver identidad (ruta flujo/cache/LPM).
3. Adjuntar metadata de mapeo para etapas posteriores.

Una dirección clave de optimización es reducir lookups repetidos:

- usar hits de hot-cache para direcciones/flujos activos
- usar LPM como fallback cuando corresponde
- evitar trabajo duplicado entre XDP y `tc` cuando la metadata puede pasarse hacia adelante

### 4.2 Por qué `cpumap` es central

`cpumap` se usa para distribuir trabajo entre núcleos y evitar que shaping se quede limitado a una sola ruta de cola. Esto es parte clave para escalar de "funciona" a "funciona a nivel ISP".

### 4.3 Cache, generaciones y reducción de presión por locks

La progresión general descrita en notas de desarrollo/devblogs:

1. reducir wipes completos de mapas
2. mover manejo de estado obsoleto hacia modelo por generación/epoch
3. reducir mantenimiento con locks pesados en hot paths

Operativamente, esto ayuda a estabilizar latencia y CPU bajo actualizaciones frecuentes.

## 5) Jerarquía de Colas: `mq` -> `HTB` -> qdisc hoja

El modelo de colas de LibreQoS es intencionalmente por capas:

1. raíz `mq` para distribución multi-cola
2. `HTB` para envolventes jerárquicas de tasa
3. qdisc hoja (`CAKE` o `fq_codel`) para equidad/AQM dentro de esas envolventes

```{mermaid}
flowchart TD
    A[qdisc raiz mq] --> B[Clase padre HTB CPU/RXQ 0]
    A --> C[Clase padre HTB CPU/RXQ 1]

    B --> D[Clase HTB de topologia: Sitio/AP/POP]
    D --> E[Clase HTB de circuito: Suscriptor/Circuito]
    E --> F[qdisc hoja: CAKE o fq_codel]

    C --> G[Clase HTB de topologia: Sitio/AP/POP]
    G --> H[Clase HTB de circuito: Suscriptor/Circuito]
    H --> I[qdisc hoja: CAKE o fq_codel]

    F --> J[Paquetes moldeados de egreso]
    I --> J
```

### 5.1 Internals de HTB que importan en producción

Mecánicas importantes:

- Tokens: cada paquete consume tokens según su tamaño.
- Refill timing: el refill de tokens sigue el timing del kernel (`jiffies`).
- `quantum`: bytes servidos antes de que el scheduler rote foco de clase.
- `r2q`: influye en los valores derivados de quantum y su comportamiento.

Por qué importa para operadores:

- valores muy pequeños o muy grandes de quantum pueden afectar la suavidad de equidad
- relaciones padre/hijo en shaping importan más que "recetas" aisladas de una sola clase
- HTB es la envolvente de tasa; los qdiscs hoja no reemplazan la política HTB

## 6) AQM en LibreQoS: `fq_codel` y `CAKE`

### 6.1 División de responsabilidades

División práctica:

1. HTB: política jerárquica de ancho de banda y límites
2. `fq_codel`/`CAKE`: equidad de cola y control de delay dentro de esa política

### 6.2 Por qué puede haber beneficios por debajo de la tasa máxima

Las mejoras de AQM/fair-queueing no dependen solo de "enlace al 100%".

Incluso cuando la interfaz agregada no está constantemente al límite, pueden aparecer mejoras visibles porque:

1. los microbursts siguen creando presión de cola
2. los flujos compiten por servicio de cola
3. el scheduler por flujo reduce dominancia de ciertos flujos y picos de espera
4. drops/marks orientados a delay evitan colas persistentes muy largas

Afirmación segura para operadores:

- "AQM puede mejorar la respuesta y consistencia de latencia bajo carga mixta real, incluyendo periodos por debajo del techo duro, según mezcla de tráfico y topología."

### 6.3 CAKE vs fq_codel en términos de LibreQoS

Patrón general:

1. Preferir CAKE cuando la prioridad es suavidad con tráfico mixto y buen comportamiento por defecto.
2. Preferir fq_codel cuando domina la presión por cantidad de colas/recursos y la QoE observada sigue siendo aceptable.
3. Revalidar tras cambios grandes de topología o cantidad de colas.

Realidad de recursos:

- ambos son flow-aware y mantienen estado
- CAKE puede tener mayor huella de memoria/CPU con poblaciones grandes de colas

### 6.4 Cuándo los beneficios bajo la tasa máxima pueden ser menores

Menor latencia bajo carga mixta es común, pero no está garantizada en todos los escenarios.

Se esperan ganancias menores cuando:

1. el cuello de botella está fuera de la ruta de cola controlada
2. el tráfico es escaso y casi no hay contención real
3. se aplica shaping en una sola dirección y el problema principal está en la dirección opuesta
4. límites de hardware fuerzan un diseño de colas con poca aislación en picos

Conclusión operativa:

- Tratar las ganancias de AQM como resultado de dinámica de colas y control de contención, y validar empíricamente con tráfico real.

## 7) Bakery y Comportamiento de Recarga

Bakery existe para evitar reconstrucciones innecesarias de colas y reducir penalidades de recarga.

Flujo general:

1. Construir estado deseado.
2. Hacer diff entre estado deseado y activo.
3. Aplicar el delta más pequeño y seguro.
4. Disparar recarga completa solo cuando el tipo de cambio cruza límites de mutación en vivo.

### 7.1 Colas perezosas y expiración

Controles clave:

1. `lazy_queues`: diferir creación de partes de la jerarquía hasta uso real.
2. `lazy_expire_seconds`: remover estado de cola inactivo tras timeout.

Efecto práctico:

- menor consumo de memoria para endpoints inactivos
- menos churn para poblaciones grandes de suscriptores parcialmente activos

### 7.2 Límite entre incremental y recarga completa

| Tipo de cambio | Normalmente incremental seguro | Suele requerir recarga completa | Por qué |
|---|---|---|---|
| Cambio solo de IP de circuito | Sí | No | Actualización de mapeo puede aplicarse sin reconstruir árbol |
| Cambio de velocidad de circuito/sitio (subconjunto) | Sí | A veces | Depende del impacto estructural y class handles disponibles |
| Cambios masivos en todos los circuitos | A veces | Frecuente | Límites de escala y cardinalidad de transacciones |
| Re-parent/reestructura topológica | Rara vez | Sí | Restricciones de mutación de subárbol HTB |
| Alta/baja de circuitos | Sí (pequeño/mediano) | A veces | Límites de handles y fronteras de corrección en el diff |

Esta tabla refleja comportamiento de diseño de Bakery y restricciones de mutación de Linux `tc` discutidas en el devblog.

```{mermaid}
flowchart TD
    A[Llega cambio de config o integracion] --> B[Construir estado deseado y calcular diff]
    B --> C{Hay cambio efectivo de estado?}
    C -->|No| D[No-op]
    C -->|Si| E{Hay cambio estructural de jerarquia?}
    E -->|Si| F[Recarga completa controlada]
    E -->|No| G{Dentro de limites incrementales seguros?}
    G -->|Si| H[Aplicar actualizaciones tc incrementales]
    G -->|No| F
    H --> I[Verificar estado y contadores de class/qdisc]
    F --> I
```

### 7.3 Reglas rápidas para límites de recarga

1. Preferir deltas pequeños y frecuentes de mapeo/velocidad sobre churn estructural grande.
2. Agrupar cirugía topológica en ventanas planificadas.
3. Esperar mayor riesgo cuando coinciden muchos circuitos y muchos cambios estructurales.
4. Diseñar la cadencia operativa para priorizar cambios incrementalmente seguros.

## 8) Límites de Diseño para Operadores

### 8.1 Límites de observabilidad

| Señal | Fuerte para | Débil para |
|---|---|---|
| Contadores de colas y métricas de shaping | Diagnóstico de tendencias, congestión y validación de política | Causalidad exacta paquete-por-paquete |
| Drops/marks de CAKE/fq_codel | Detectar presión persistente de cola y efectos de política | Asignar culpa end-to-end a nivel aplicación |
| CPU/RAM y tiempos de comando | Planificación de capacidad y riesgo de recarga | Aislar origen exacto de cada microburst |

### 8.2 Factores de riesgo de capacidad y mitigaciones

| Factor de riesgo | Síntoma típico | Mitigación |
|---|---|---|
| Cantidad muy alta de colas con CAKE en todas | Crecimiento de RAM y overhead del scheduler | Usar `lazy_queues`, expiración, y fq_codel selectivo cuando aplique |
| Actualizaciones frecuentes de árbol completo | Ventanas breves de disrupción de paquetes | Aumentar uso de cambios incrementalmente seguros; agrupar cambios estructurales |
| Mapeo de padre incompleto en jerarquía | Suscriptores sin shaping esperado | Validar relaciones de padre en `network.json` y en inputs |
| Virtualización/NIC con colas débiles o una sola cola | Mala distribución y shaping inestable | Asegurar ruta multi-cola y verificar supuestos de mapeo |

## 9) Matriz de Troubleshooting: Síntoma a Causa

| Síntoma | Causa backend común | Primeras verificaciones | Dirección correctiva típica |
|---|---|---|---|
| Picos de latencia sin throughput totalmente al límite | Microbursts, aislación de flujo insuficiente, o mismatch direccional | Comparar latencia vs tendencias de cola/drop; verificar shaping en ambas direcciones | Ajustar estrategia de qdisc hoja y diseño direccional |
| Un CPU muy caliente y otros subutilizados | Desbalance de steering o ruta multi-cola débil | Revisar uso de CPU y contadores por rama de cola | Corregir supuestos de mapeo y verificar estructura `mq`/clases |
| Suscriptores aparecen sin shaping intermitentemente | Mismatch en mapeo padre/jerarquía | Validar referencias de nodo padre y creación de clases | Corregir jerarquía, aplicar y verificar presencia de clases |
| Disrupciones cortas frecuentes durante cambios | Demasiados cambios que disparan recarga completa | Clasificar cambios recientes: estructural vs incremental | Reagrupar operaciones hacia deltas incrementales seguros |
| Crecimiento de RAM al escalar | Demasiados qdisc hoja activos o huella CAKE agresiva | Medir cantidad de colas y tendencia de memoria en ventanas de cambio | Usar colas perezosas/expiración y considerar fq_codel selectivo |
| Dashboard muestra más tráfico del esperado | Alcance del contador difiere de tráfico final post-drop | Comparar métricas de dashboard con contexto `tc` drop/mark | Ajustar runbooks a semántica de métricas |

## 10) Flujo de Validación de Cambios

Usa este flujo ligero para cambios con impacto backend.

### 10.1 Antes del cambio

1. Clasificar tipo de cambio: mapeo/velocidad/estructura.
2. Estimar alcance: cantidad de circuitos/clases afectados.
3. Capturar línea base:
   - tendencia de latencia
   - comportamiento de drop/mark
   - margen de CPU y RAM
   - snapshot de clases/qdiscs `tc`

### 10.2 Durante el cambio

1. Vigilar comportamiento del plano de control:
   - ocurrencia de aplicación incremental vs recarga completa
   - warnings/errores de comandos y runtime
2. Vigilar señales del plano de datos:
   - anomalías de crecimiento de cola
   - deriva de latencia por dirección
   - discontinuidades de contadores por clase

### 10.3 Después del cambio

1. Revalidar las mismas señales de la línea base.
2. Confirmar presencia de jerarquía/clases en circuitos modificados.
3. Verificar expectativas de latencia y throughput del suscriptor.
4. Si hay degradación, revertir o reducir alcance y reaplicar en lotes más pequeños.

### 10.4 Checklist mínimo de comandos

Ajusta nombres de interfaz a tu entorno.

```bash
tc -s qdisc show dev <ifname>
tc -s class show dev <ifname>
journalctl -u lqosd --since "15 min ago"
```

## 11) Secuencia Práctica de Ajuste

Orden recomendado:

1. Validar primero jerarquía topológica y mapeos de padre.
2. Confirmar cantidad de colas y margen de memoria.
3. Validar comportamiento de distribución `mq`/multi-core.
4. Elegir CAKE vs fq_codel según QoE observada y presupuesto de recursos.
5. Ajustar cadencia de cambios para favorecer deltas incrementalmente seguros.
6. Revalidar tras cambios grandes de plan de velocidad, topología o cadencia de integración.

## 12) Glosario

- `XDP`: hook de paquetes de alto rendimiento más temprano en Linux.
- `eBPF`: procesamiento programable en kernel.
- `LPM`: lookup de prefijo más largo para mapeo de identidad.
- `cpumap`: mapa XDP para dirigir procesamiento a CPUs.
- `tc`: subsistema Linux Traffic Control.
- `qdisc`: objeto de disciplina de cola en `tc`.
- `mq`: estructura raíz multi-cola.
- `HTB`: scheduler/shaper Hierarchical Token Bucket.
- `fq_codel`: fair queueing + control de delay CoDel.
- `CAKE`: qdisc integrado de shaping/equidad/AQM.
- `Bakery`: subsistema LibreQoS de diff de estado y actualización incremental.
- `epoch/generation`: enfoque de envejecimiento de estado para evitar limpiezas globales con locks pesados.

## Lectura Relacionada

- [CAKE](cake-es.md)
- [Ajuste de Rendimiento](performance-tuning-es.md)
- [Planificación de Escala y Diseño de Topología](scale-topology-es.md)
- [Recetas de Despliegue](recipes-es.md)
- [Introducing the LibreQoS Bakery](https://devblog.libreqos.com/posts/0005-lqos-bakery/)
- [Fixing the Reload Penalty in LibreQoS](https://devblog.libreqos.com/posts/0013-no-more-locks/)
