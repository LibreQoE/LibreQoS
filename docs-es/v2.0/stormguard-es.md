# StormGuard

StormGuard es el subsistema de ajuste adaptativo de colas de LibreQoS para eventos de congestión y calidad.

## Qué hace StormGuard

- Monitorea señales en tiempo real (throughput, métricas RTT/pérdida y contexto de saturación).
- Aplica ajustes acotados a límites de nodos configurados para proteger calidad bajo estrés.
- Expone estado y depuración en WebUI.

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

## UI y depuración

- WebUI (Node Manager) incluye vistas de estado y depuración de StormGuard.
- La página de depuración muestra:
  - límites efectivos actuales
  - métricas de evaluación
  - contexto de reglas/decisiones

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
- que `log_file` (si se usa) sea escribible por el servicio

## Páginas relacionadas

- [Configuración](configuration-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Ajuste de rendimiento](performance-tuning-es.md)
- [Alta Disponibilidad y Dominios de Falla](high-availability-es.md)
- [Componentes](components-es.md)
- [Solución de Problemas](troubleshooting-es.md)
