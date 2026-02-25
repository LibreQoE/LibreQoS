# Componentes de software de LibreQoS

## Servicios systemd

### lqosd

- Administra la lógica de shaping/XDP.
- Implementado en Rust.
- Ejecuta la UI local disponible en `http://a.b.c.d:9123`.
- Hospeda páginas de Node Manager como:
  - Mapa de flujos
  - Árbol de red
  - Explorador ASN
  - Árbol/Pesos de CPU
  - Editores de configuración para integraciones (UISP, Splynx, Netzur, VISP, etc.)

### lqos_scheduler

- Realiza refrescos continuos de shaping, incluyendo lectura de integraciones CRM habilitadas.
- Acciones:
  - Al iniciar: ejecución de setup completo de colas.
  - Cada X minutos: actualización de colas y datos de integración.
- El intervalo por defecto es 30 minutos (`queue_refresh_interval_mins` en `/etc/lqos.conf`).

### Verificación de estado

```bash
sudo systemctl status lqosd lqos_scheduler
```

Si algún servicio aparece como `failed`, revise logs:

```bash
sudo journalctl -u lqosd -b
sudo journalctl -u lqos_scheduler -b
```

### Depuración de lqos_scheduler

`lqos_scheduler` ejecuta `scheduler.py`, que a su vez invoca `LibreQoS.py`.

Flujo de depuración recomendado:

```bash
sudo systemctl stop lqos_scheduler
cd /opt/libreqos/src
sudo ./LibreQoS.py --debug
sudo python3 scheduler.py
sudo systemctl start lqos_scheduler
```

## Modo privacidad en Node Manager

Node Manager incluye redacción del lado cliente para demos/capturas:

- Se activa con el ícono de máscara en la barra superior.
- La preferencia se guarda en `localStorage`.
- Oculta datos visibles en la UI; no modifica archivos fuente.

## Canal de problemas urgentes

Node Manager incluye un canal de problemas urgentes para eventos de alta prioridad (por ejemplo, límite de circuitos mapeados).

- Los eventos aparecen en el indicador de navegación superior.
- Pueden revisarse y reconocerse desde el modal de problemas urgentes.
- Úselo como señal rápida; confirme detalle con `journalctl -u lqosd`.

## Indicador de estado del scheduler

Node Manager incluye visibilidad del estado del scheduler.

- Úselo como señal rápida de salud para jobs periódicos.
- Si el scheduler no está saludable, valide primero estado de `lqosd` y `lqos_scheduler`.
- Revise detalle con:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - `journalctl -u lqosd --since "30 minutes ago"`

## Solución de problemas de componentes

Consulte [Solución de Problemas](troubleshooting-es.md).
