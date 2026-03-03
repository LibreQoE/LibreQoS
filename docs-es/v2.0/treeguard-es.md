# TreeGuard (Función Próxima de v2.0)

TreeGuard es una función próxima de LibreQoS v2.0 para gestión inteligente de nodos.

Estado importante:

1. TreeGuard es **próximo**.
2. TreeGuard **no está habilitado por defecto**.
3. Los valores por defecto actuales no cambian a menos que un operador habilite TreeGuard explícitamente.

## Qué Hace TreeGuard

TreeGuard tiene dos dominios de control:

1. Gestión de virtualización de enlaces/nodos (para nodos seleccionados).
2. Conmutación de SQM por circuito entre `cake` y `fq_codel`.

Para circuitos, TreeGuard puede tomar decisiones por dirección (descarga y subida de forma independiente).

## Comportamiento por Defecto sin TreeGuard

Cuando TreeGuard no está habilitado, LibreQoS mantiene la política SQM configurada/global (normalmente `cake diffserv4`, con overrides del operador cuando estén configurados).

TreeGuard no afecta el tráfico si no está habilitado.

## Modelo de Conmutación SQM por Circuito (Cuando Está Habilitado)

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

Valores por defecto iniciales (PR #946):

```toml
[treeguard]
enabled = false
dry_run = true
tick_seconds = 1

[treeguard.circuits]
enabled = true
switching_enabled = true
independent_directions = true
idle_util_pct = 2.0
idle_min_minutes = 15
rtt_missing_seconds = 120
upgrade_util_pct = 5.0
min_switch_dwell_minutes = 30
max_switches_per_hour = 4
persist_sqm_overrides = true
```

## Patrón de Despliegue Seguro

1. Mantenga `enabled = false` hasta revisar política y listas de enrolamiento.
2. Inicie con `enabled = true` y `dry_run = true`.
3. Use primero una allowlist pequeña de nodos/circuitos.
4. Valide el comportamiento en varias ventanas pico y valle.
5. Solo entonces establezca `dry_run = false`.

## Overrides y Notas Operativas

Cuando está habilitado y no está en dry-run, TreeGuard puede persistir decisiones en:

- `lqos_overrides.treeguard.json`

TreeGuard está diseñado para no pelear con overrides del operador. Si existen overrides del operador para entidades enroladas, TreeGuard omite esas entidades y reporta advertencias.

## Páginas Relacionadas

- [HTB + fq_codel + CAKE: Comportamiento Detallado de Colas](htb_fq_codel_cake-es.md)
- [Referencia de Configuración Avanzada](configuration-advanced-es.md)
- [Arquitectura Backend de LibreQoS](libreqos-backend-architecture-es.md)
- [Insumos para Desarrollo Futuro](future-development-inputs-es.md)
