# TreeGuard

TreeGuard es una función actual de LibreQoS v2.0 para gestión inteligente de nodos.

Estado importante:

1. TreeGuard está **habilitado por defecto** en LibreQoS v2.0.
2. TreeGuard puede gestionar tanto la virtualización de nodos elegibles como la política SQM por circuito.
3. Los operadores pueden ajustar o deshabilitar TreeGuard en `/etc/lqos.conf` o en la página TreeGuard de la WebUI.

## Qué Hace TreeGuard

TreeGuard tiene dos dominios de control:

1. Gestión de virtualización de enlaces/nodos (para nodos seleccionados).
2. Conmutación de SQM por circuito entre `cake` y `fq_codel`.

Para circuitos, TreeGuard puede tomar decisiones por dirección (descarga y subida de forma independiente).

## Comportamiento por Defecto en LibreQoS v2.0

En LibreQoS v2.0, TreeGuard está habilitado por defecto.

Por defecto, TreeGuard puede virtualizar nodos enrolados y puede cambiar direcciones de circuitos enrolados entre `cake diffserv4` y `fq_codel` según los guardrails configurados.

Si prefiere un comportamiento fijo/manual, deshabilite TreeGuard o reduzca sus listas de enrolamiento.

## Modelo de Conmutación SQM por Circuito

TreeGuard evalúa utilización, frescura de RTT, guardrails de CPU y guardrails opcionales de QoE.

Comportamiento de alto nivel:

1. Bajo condiciones sostenidas de baja carga, TreeGuard puede cambiar una dirección de `cake` a `fq_codel`.
2. Si sube la utilización, los guardrails de QoE no son seguros o se cumplen condiciones de reversión, TreeGuard vuelve hacia la política SQM base del circuito.
3. Las decisiones pueden ser independientes por dirección cuando `independent_directions = true`.

Esto crea un perfil dinámico en el que direcciones cargadas favorecen `cake diffserv4`, mientras direcciones de baja carga pueden usar `fq_codel` cuando las condiciones son seguras.

La política SQM base proviene de la intención del operador, no de defaults internos de TreeGuard. En la práctica, TreeGuard parte de la política efectiva configurada para cada circuito y solo persiste su propio overlay temporal cuando necesita diferir de esa base.

Regla importante de política base:

1. Si la política SQM base de una dirección es `cake`, TreeGuard puede cambiar temporalmente esa dirección a `fq_codel` y luego volver a la base.
2. Si la política SQM base de una dirección es `fq_codel`, TreeGuard no conmuta esa dirección de circuito hacia `cake`.
3. La virtualización de enlaces sigue disponible independientemente de la política SQM base del circuito.

## Configuración (`/etc/lqos.conf`)

La configuración de TreeGuard vive bajo `[treeguard]` y sus sub-secciones:

1. `[treeguard]`: habilitar/deshabilitar, dry-run, cadencia de ticks.
2. `[treeguard.cpu]`: modo basado en CPU vs tráfico/RTT y umbrales.
3. `[treeguard.links]`: enrolamiento de virtualización de nodos y guardrails.
4. `[treeguard.circuits]`: enrolamiento de circuitos y guardrails de conmutación SQM.
5. `[treeguard.qoo]`: umbral opcional de protección QoE.

Comportamiento por defecto actual:

```toml
[treeguard]
enabled = true
dry_run = false
tick_seconds = 1

[treeguard.cpu]
mode = "cpu_aware"
cpu_high_pct = 75
cpu_low_pct = 55

[treeguard.links]
enabled = true
all_nodes = true
top_level_auto_virtualize = true

[treeguard.circuits]
enabled = true
all_circuits = true
switching_enabled = true
independent_directions = true

[treeguard.qoo]
enabled = true
```

La virtualización de nodos en TreeGuard está pensada para ser basada en CPU por defecto. El
tráfico, RTT y QoE siguen siendo señales importantes de seguridad y restauración, pero la nueva
virtualización automática debe ocurrir cuando la presión de CPU indica que el ahorro de HTB vale
la pena. Las instalaciones actualizadas desde defaults antiguos se migran silenciosamente de
`traffic_rtt_only` a `cpu_aware`, con un aviso visible en logs/UI.

## Patrón de Despliegue Seguro

1. Revise la configuración de TreeGuard temprano en el despliegue en lugar de asumir comportamiento de colas fijo/manual.
2. Si desea un rollout más acotado, deshabilite `all_nodes` y/o `all_circuits` y utilice allowlists primero.
3. Valide el comportamiento en varias ventanas pico y valle.
4. Si quiere validar solo en modo observación, establezca `dry_run = true` temporalmente.
5. Si necesita comportamiento fijo/manual, establezca `enabled = false`.

## Overrides y Notas Operativas

Cuando está habilitado y no está en dry-run, TreeGuard puede persistir decisiones SQM de circuitos en:

- `lqos_overrides.treeguard.json`

TreeGuard está diseñado para no pelear con overrides del operador. Si existen overrides del operador para entidades enroladas, TreeGuard omite esas entidades y reporta advertencias.

Las decisiones de virtualización de nodos de TreeGuard son operaciones de Bakery solo en tiempo de
ejecución. El scheduler no las materializa de vuelta en el `network.json` base, y tampoco se
persisten como entradas TreeGuard `set_node_virtual` dentro de la entrada efectiva de shaping. En
v1 son efímeras: un reinicio del daemon devuelve el árbol físico a la topología base definida por
el operador hasta que TreeGuard vuelva a decidir.

Para verificación y depuración local en tiempo de ejecución, `liblqos_python` ahora expone tanto
el estado actual de la operación del nodo TreeGuard como un snapshot del estado de ramas en tiempo
de ejecución de Bakery. Ese snapshot de ramas es la vista autoritativa del plano de control sobre
qué rama retenida está activa para un nodo, y es más confiable que inferir cambios de parentaje
solo desde la salida de `tc` en casos no top-level.
El flujo local de confianza también usa ahora un fixture sintético de Bakery TreeGuard más grande
para pruebas de escala: 8 nodos top-level, 3 niveles de profundidad y 1.000 circuitos conectados
solo en el nivel más bajo. Cuando la verificación con tráfico está habilitada, el runtime verifier
ahora usa por defecto 10 circuitos rastreados por cada caso exitoso en lugar del smoke test menor
anterior.

Las decisiones SQM por circuito de TreeGuard también son overrides de tiempo de ejecución. El scheduler no materializa los cambios SQM propiedad de TreeGuard de vuelta en el `ShapedDevices.csv` base, por lo que limpiar TreeGuard no reescribe permanentemente la política SQM definida por el operador.

TreeGuard también se niega a gestionar nodos que ya estén marcados con `"virtual": true` en el `network.json` base. Si existen overrides legados de TreeGuard para esos nodos, TreeGuard limpia ese estado legado y vuelve a respetar la definición base de la topología.

Para la gestión SQM por circuito, TreeGuard trata los valores duplicados de `device_id` como colisiones de identidad inseguras. Si el mismo `device_id` aparece en más de un circuito dentro de `ShapedDevices.csv`, TreeGuard omite esos circuitos afectados y limpia cualquier override SQM de TreeGuard asociado a esos `device_id` duplicados.

Si la telemetría RTT no está disponible temporalmente después de un reinicio, TreeGuard no trata la ausencia de RTT por sí sola como evidencia para revertir direcciones en `fq_codel`. Siguen aplicando otros guardrails como utilización, QoE y presión de CPU.

TreeGuard también aplica un presupuesto global conservador de cambios SQM por tick. En poblaciones muy grandes de circuitos enrolados, los cambios SQM excedentes se difieren a ticks posteriores en lugar de saturar Bakery en una sola pasada.

Para escalar mejor, TreeGuard ya no reconstruye la membresía de circuitos desde `ShapedDevices.csv` en cada tick de circuitos. Ahora mantiene un inventario por circuito en caché derivado de `ShapedDevices.csv`, lee la telemetría viva por circuito desde el snapshot compartido de rollup por circuito actualizado una vez por segundo, y reparte las evaluaciones SQM grandes con `all_circuits = true` a lo largo de múltiples ticks en lugar de reescanear todos los circuitos enrolados cada segundo.

En la práctica esto significa:

1. La virtualización de enlaces sigue la cadencia normal de ticks de TreeGuard.
2. La evaluación SQM de circuitos para enrolamientos pequeños sigue completándose rápidamente.
3. Los enrolamientos muy grandes con `all_circuits` se recorren de forma incremental a lo largo de varios ticks, con un objetivo de barrido completo de alrededor de 15 segundos en vez de intentar un escaneo completo por segundo.
4. La virtualización de nodos de TreeGuard ahora usa rutas vivas de planificación/aplicación en Bakery en lugar de forzar una recarga completa de LibreQoS o de Bakery.
5. La virtualización runtime soportada de nodos top-level ahora usa un plan de rebalanceo/migración en Bakery que puede promover sitios hijos y circuitos directos entre raíces de cola, preservando la jerarquía lógica para reportes.

La actividad reciente de TreeGuard está disponible en dos lugares:

- Las vistas de estado/actividad de TreeGuard en la WebUI.
- El journal de `lqosd`, donde TreeGuard ahora registra cada evento de actividad para que recargas, limpieza de overrides, cambios de SQM y fallos se puedan diagnosticar sin inspeccionar websockets.

## Páginas Relacionadas

- [HTB + fq_codel + CAKE: Comportamiento Detallado de Colas](htb_fq_codel_cake-es.md)
- [Referencia de Configuración Avanzada](configuration-advanced-es.md)
- [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md)
- [Insumos para Desarrollo Futuro](future-development-inputs-es.md)
