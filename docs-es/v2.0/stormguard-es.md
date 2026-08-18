# StormGuard

StormGuard es el subsistema de ajuste adaptativo de colas de LibreQoS para eventos de congestión y calidad.

> **Advertencia importante de alcance**
> StormGuard está pensado para casos de uso específicos, como controlar congestión en enlaces WAN de ancho de banda variable (por ejemplo redes marítimas), o para un número pequeño de puntos de acceso con capacidades muy variables.
> No está pensado para gestionar decenas o cientos de nodos al mismo tiempo.

## Qué hace StormGuard

- Monitorea señales en tiempo real (throughput, métricas RTT/pérdida y contexto de saturación).
- Aplica ajustes acotados a límites de nodos configurados para proteger calidad bajo estrés.
- Expone estado y depuración en WebUI.

Los cambios adaptativos de velocidad por sitio de StormGuard se guardan en la capa de overrides de StormGuard. No se escriben de vuelta en `network.json`.

## Configuración

StormGuard se configura en `/etc/lqos.conf` bajo `[stormguard]`.

Claves comunes:

- `enabled`: habilita o deshabilita StormGuard.
- `dry_run`: calcula decisiones sin aplicar cambios de colas en vivo.
- `targets`: lista de nodos de nivel superior a gestionar.
- `minimum_download_percentage`: piso mínimo de descarga.
- `minimum_upload_percentage`: piso mínimo de subida.
- `log_file`: ruta opcional para telemetría CSV de decisiones/cambios.

Ejemplo:

```toml
[stormguard]
enabled = true
dry_run = true
log_file = "/var/log/stormguard.csv"
targets = ["SITE_A", "SITE_B"]
minimum_download_percentage = 0.5
minimum_upload_percentage = 0.5
```

Si está probando, comience con `dry_run = true`.

Al deshabilitar StormGuard, o al volver a `dry_run = true` después de usarlo en modo activo, las colas administradas recuperan sus tasas garantizadas y límites máximos configurados, y se eliminan los ajustes adaptativos persistidos por StormGuard. Los ajustes administrados por el operador no se modifican. Durante el arranque, esta limpieza puede ejecutarse antes de que Bakery termine la inicialización normal de colas, pero solo para clases activas que coincidan con el registro persistido de propiedad de StormGuard y con la generación actual del árbol. La limpieza espera durante una recarga completa y conserva el registro de propiedad hasta que Bakery confirma la restauración.

## UI y depuración

- WebUI (Node Manager) incluye una pestaña dedicada de StormGuard además de las vistas de estado y depuración.
- La pestaña del dashboard está pensada para responder "qué está haciendo StormGuard ahora mismo?" con:
  - tarjetas resumen para sitios observados, en cooldown y con cambios recientes
  - una lista de sitios que funciona tanto con un único sitio observado como con conjuntos más grandes
  - un panel de detalle por sitio seleccionado que explica límites actuales, últimas acciones y por qué StormGuard mantiene o cambia velocidades
  - un feed de actividad reciente para triage rápido del operador
- La página de depuración muestra:
  - límites efectivos actuales
  - métricas de evaluación
  - contexto de reglas/decisiones
- La página **Árbol de red** muestra una pestaña contextual **StormGuard** mientras StormGuard esté habilitado o quede un estado de limpieza/degradación. Al seleccionar un nodo observado se muestran sus límites actuales de descarga/subida, rangos, estrategia, cooldown, motivo de decisión, último resultado y un gráfico local del navegador de cinco minutos. La pestaña indica si el nodo seleccionado no está administrado y enlaza con la configuración de StormGuard para realizar cambios.

El estado de ejecución usa una de estas fases:

- `disabled`: StormGuard está deshabilitado y no queda limpieza pendiente.
- `initializing`: la configuración, la topología o las dependencias de Bakery aún no están listas.
- `dry_run`: se evalúan decisiones sin modificar colas activas.
- `live`: se permiten ajustes activos confirmados.
- `cleanup_pending`: todavía debe restaurarse estado de colas propiedad de StormGuard.
- `degraded`: un error impide la evaluación o limpieza normal; revise el último error mostrado y el registro del servicio.

## Registro de diagnóstico

Cuando se configura `log_file`, StormGuard añade cada segundo una fila delimitada por punto y coma por sitio observado y dirección. El primer campo contiene la versión del esquema. La cabecera de la versión 1 es:

```text
schema_version;timestamp_unix_ms;site;direction;mode;strategy;queue_mbps;min_mbps;max_mbps;throughput_mbps;throughput_ma_mbps;retransmit_fraction;retransmit_ma;passive_rtt_ms;active_ping_rtt_ms;active_ping_target;active_ping_weight;effective_rtt_ms;rtt_ma_ms;baseline_rtt_ms;delay_ms;passive_rtt_flow_count;decision_score;candidate_action;candidate_target_mbps;decision_reason;decision_blocker;state;cooldown_remaining_secs;last_attempt_action;last_attempt_target_mbps;last_attempt_outcome;last_attempt_unix_ms;last_attempt_error;rtt_source
```

Los valores no disponibles se escriben como campos vacíos. El archivo se conserva y se amplía entre reinicios del daemon; la cabecera solo se escribe para un archivo nuevo o vacío. Al alcanzar 64 MiB, StormGuard rota el archivo a `<log_file>.1` y reemplaza la copia `.1` anterior, por lo que se conserva como máximo una copia.

Los resultados de aplicación distinguen `applied`, `dry_run`, `skipped` y `failed`. Un ajuste fallido no cambia el límite actual de StormGuard ni inicia el cooldown, de modo que puede volver a intentarse.

## Patrón de despliegue seguro

1. Habilitar con `dry_run = true`.
2. Observar durante varios periodos pico.
3. Validar que no haya oscilaciones indeseadas.
4. Cambiar a `dry_run = false`.
5. Monitorear después de cada cambio grande de topología/integración.

## Solución de problemas

Si el comportamiento parece incorrecto:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd --since "30 minutes ago"
```

También verifique:

- que los nombres en `targets` aún coincidan con `network.json`
- que cambios de integración no hayan renombrado nodos clave
- que los pisos mínimos sean razonables para su perfil de tráfico
- que `network.json` siga reflejando sus velocidades planificadas/de origen si está investigando una reducción inesperada de StormGuard
- que `log_file` (si se usa) sea escribible por el servicio

## Páginas relacionadas

- [Configuración](configuration-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Ajuste de rendimiento](performance-tuning-es.md)
- [Alta Disponibilidad y Dominios de Falla](high-availability-es.md)
- [Componentes](components-es.md)
- [Solución de Problemas](troubleshooting-es.md)
