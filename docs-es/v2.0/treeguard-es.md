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

TreeGuard evalúa utilización, frescura de RTT, guardrails de CPU y guardrails opcionales de QoO.

Comportamiento de alto nivel:

1. Bajo condiciones sostenidas de baja carga, TreeGuard puede cambiar una dirección de `cake` a `fq_codel`.
2. Si sube la utilización, los guardrails RTT/QoO no son seguros o se cumplen condiciones de reversión, TreeGuard vuelve a `cake`.
3. Las decisiones pueden ser independientes por dirección cuando `independent_directions = true`.

Esto crea un perfil dinámico en el que direcciones cargadas favorecen `cake diffserv4`, mientras direcciones de baja carga pueden usar `fq_codel` cuando las condiciones son seguras.

## Configuración (`/etc/lqos.conf`)

La configuración de TreeGuard vive bajo `[treeguard]` y sus sub-secciones:

1. `[treeguard]`: habilitar/deshabilitar, dry-run, cadencia de ticks.
2. `[treeguard.cpu]`: modo basado en CPU vs tráfico/RTT y umbrales.
3. `[treeguard.links]`: enrolamiento de virtualización de nodos y guardrails.
4. `[treeguard.circuits]`: enrolamiento de circuitos y guardrails de conmutación SQM.
5. `[treeguard.qoo]`: umbral opcional de protección QoO.

Comportamiento por defecto actual:

```toml
[treeguard]
enabled = true
dry_run = false
tick_seconds = 1

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

## Patrón de Despliegue Seguro

1. Revise la configuración de TreeGuard temprano en el despliegue en lugar de asumir comportamiento de colas fijo/manual.
2. Si desea un rollout más acotado, deshabilite `all_nodes` y/o `all_circuits` y utilice allowlists primero.
3. Valide el comportamiento en varias ventanas pico y valle.
4. Si quiere validar solo en modo observación, establezca `dry_run = true` temporalmente.
5. Si necesita comportamiento fijo/manual, establezca `enabled = false`.

## Overrides y Notas Operativas

Cuando está habilitado y no está en dry-run, TreeGuard puede persistir decisiones en:

- `lqos_overrides.treeguard.json`

TreeGuard está diseñado para no pelear con overrides del operador. Si existen overrides del operador para entidades enroladas, TreeGuard omite esas entidades y reporta advertencias.

Las decisiones de virtualización de nodos de TreeGuard son overrides de tiempo de ejecución. El
scheduler no las materializa de vuelta en el `network.json` base, por lo que deshabilitar o limpiar
TreeGuard no reescribe permanentemente la topología definida por el operador.

TreeGuard también se niega a gestionar nodos que ya estén marcados con `"virtual": true` en el `network.json` base. Si existen overrides viejos de TreeGuard para esos nodos, TreeGuard limpia su propia capa de overrides y vuelve a respetar la definición base de la topología.

Para la gestión SQM por circuito, TreeGuard trata los valores duplicados de `device_id` como colisiones de identidad inseguras. Si el mismo `device_id` aparece en más de un circuito dentro de `ShapedDevices.csv`, TreeGuard omite esos circuitos afectados y limpia cualquier override SQM de TreeGuard asociado a esos `device_id` duplicados.

La actividad reciente de TreeGuard está disponible en dos lugares:

- Las vistas de estado/actividad de TreeGuard en la WebUI.
- El journal de `lqosd`, donde TreeGuard ahora registra cada evento de actividad para que recargas, limpieza de overrides, cambios de SQM y fallos se puedan diagnosticar sin inspeccionar websockets.

## Páginas Relacionadas

- [HTB + fq_codel + CAKE: Comportamiento Detallado de Colas](htb_fq_codel_cake-es.md)
- [Referencia de Configuración Avanzada](configuration-advanced-es.md)
- [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md)
- [Insumos para Desarrollo Futuro](future-development-inputs-es.md)
