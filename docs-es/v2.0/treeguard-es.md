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

## Páginas Relacionadas

- [HTB + fq_codel + CAKE: Comportamiento Detallado de Colas](htb_fq_codel_cake-es.md)
- [Referencia de Configuración Avanzada](configuration-advanced-es.md)
- [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md)
- [Insumos para Desarrollo Futuro](future-development-inputs-es.md)
