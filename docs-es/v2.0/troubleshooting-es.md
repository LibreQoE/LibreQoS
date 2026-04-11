# Solución de problemas

## Empiece aquí: triage por síntoma

Use esta tabla para ir al primer check rápidamente.

¿Necesita definiciones de términos de licencia/scheduler? Vea el [Glosario](glossary-es.md).

| Síntoma | Primer check | Ubicación en WebUI | Siguiente sección |
|---|---|---|---|
| No se puede acceder a la WebUI | `systemctl status lqosd` | N/A (UI no disponible) | No hay WebUI en x.x.x.x:9123 |
| Hay tráfico pero no hace shaping | verificar `to_internet` / `to_network` y servicios | WebUI Dashboard | LibreQoS está en ejecución, pero no hace shaping |
| Scheduler no saludable | revisar logs de `lqosd` y `lqos_scheduler` | WebUI -> Scheduler Status | El estado del scheduler en WebUI aparece no saludable |
| Vistas de topología/flujo vacías | confirmar tráfico reciente y estado de `lqosd` | WebUI -> Flow Globe / Tree / ASN Analysis | Flow Globe / Tree Overview / ASN Analysis aparecen en blanco |
| Aparece código urgente | abrir detalle y mapear código | WebUI -> Urgent Issues | Códigos de problemas urgentes y primeras acciones |
| Eventos de límite de circuitos | validar licencia y conteos mapped | Insight UI + WebUI -> Urgent Issues | Se alcanzó el límite de circuitos mapeados |

## Problemas comunes

### Dónde en la WebUI

- Estado de servicios y salud general: `WebUI -> Dashboard`
- Estado/readiness del scheduler: `WebUI -> Scheduler Status`
- Alertas prioritarias: `WebUI -> Urgent Issues`
- Visualización de topología/tráfico: `WebUI -> Network Tree Overview` y `Flow Globe`
- Revisión de datos de shaping: `WebUI -> Shaped Devices Editor`

### Antes de pedir ayuda en chat: recolecte esta evidencia

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd --since "30 minutes ago"
journalctl -u lqos_scheduler --since "30 minutes ago"
```

Si el problema es de integración, agregue:

```bash
ls -lh /opt/libreqos/src/topology_import.json /opt/libreqos/src/shaping_inputs.json
```

Si usa un despliegue manual o con archivos propios en lugar de una integración incluida, agregue:

```bash
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```

Incluya también:
- versión/build actual
- tipo de integración y estrategia
- síntoma exacto y hora de inicio

### La contraseña de usuario no funciona

Los builds actuales:
- migran automáticamente archivos de autenticación antiguos
- redirigen `/login.html` a `/first-run.html` cuando no existen usuarios

```bash
sudo systemctl restart lqosd
```

Si el usuario/contraseña correctos siguen fallando, pruebe primero con ese reinicio.

Elimine `lqusers.toml` solo si desea reiniciar el acceso o si el archivo está corrupto y no se puede reparar. Después de eliminarlo, reinicie `lqosd` y abra `IP_CAJA:9123/login.html`; la WebUI debería redirigirlo automáticamente al flujo de primer inicio.

### No hay WebUI en x.x.x.x:9123

La WebUI depende de `lqosd`. En builds actuales, la mayoría de fallas de acceso WebUI se explican por `lqosd` no saludable.

```bash
sudo systemctl status lqosd
```

Luego siga el flujo completo en **El servicio lqosd no se ejecuta o falla al iniciar**.

### LibreQoS está en ejecución, pero no hace shaping

Verifique en `/etc/lqos.conf` que `to_internet` y `to_network` estén correctos.

```bash
sudo systemctl restart lqosd lqos_scheduler
sudo systemctl status lqosd lqos_scheduler
```

### On-a-stick: shaping incorrecto o una dirección débil

On-a-stick depende de split correcto por dirección. Si la detección TX o `override_available_queues` está mal, el mapeo puede degradarse.

```bash
sudo systemctl status lqosd
journalctl -u lqosd --since "10 minutes ago"
sudo systemctl restart lqosd lqos_scheduler
```

### El servicio lqosd no se ejecuta o falla al iniciar

```bash
sudo systemctl status lqosd
journalctl -u lqosd --since "10 minutes ago"
```

### Depuración avanzada de lqosd

```bash
sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd
```

### El servicio lqos_scheduler muestra errores

```bash
sudo journalctl -u lqos_scheduler --since "1 day ago" --no-pager > lqos_sched_log.txt
```

Si el scheduler falla inmediatamente después de un reinicio con un mensaje como `Socket (typically /run/lqos/bus) not found`, eso indica que `lqosd` todavía no había terminado de enlazar el bus local. Los builds actuales esperan brevemente la disponibilidad del bus al arrancar el scheduler en lugar de abortar de inmediato, por lo que ya no deberían aparecer panics repetidos de arranque tras un reinicio.

### El estado del scheduler en WebUI aparece no saludable

Versiones recientes muestran estado/readiness del scheduler en WebUI.

Si aparece caído/desactualizado:

1. Verifique ambos servicios.
2. Revise logs recientes del scheduler.
3. Revise logs de `lqosd` para eventos de scheduler ready/error.
4. Si hubo cambios recientes, reinicie servicios.

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
journalctl -u lqosd --since "30 minutes ago"
sudo systemctl restart lqosd lqos_scheduler
```

Si oscila entre ready/error, valide credenciales y timeouts de integración en `/etc/lqos.conf`.

### RTNETLINK answers: Invalid argument

Suele indicar que no se pudo agregar correctamente qdisc MQ en la NIC (colas RX/TX insuficientes). Verifique [NICs recomendadas](requirements-es.md).

### Python ModuleNotFoundError en Ubuntu 24.04

```bash
pip uninstall binpacking --break-system-packages --yes
sudo pip uninstall binpacking --break-system-packages --yes
sudo pip install binpacking --break-system-packages
pip uninstall apscheduler --break-system-packages --yes
sudo pip uninstall apscheduler --break-system-packages --yes
sudo pip install apscheduler --break-system-packages
pip uninstall deepdiff --break-system-packages --yes
sudo pip uninstall deepdiff --break-system-packages --yes
sudo pip install deepdiff --break-system-packages
```

### Todas las IPs de clientes aparecen como Unknown IPs

```bash
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo python3 LibreQoS.py
```

Corrija errores en `ShapedDevices.csv` y/o `network.json`, luego:

```bash
sudo systemctl start lqos_scheduler
```

### Flow Globe / Tree Overview / ASN Analysis aparecen en blanco

Algunas vistas requieren suficiente dato reciente para renderizar.

1. Confirme que `lqosd` está saludable.
2. Espere acumulación de tráfico.
3. Recargue la página tras 1-2 minutos.
4. Revise logs:

```bash
journalctl -u lqosd --since "10 minutes ago"
```

Si sigue en blanco con tráfico normal, recolecte logs y abra issue.

### Colisión de promoción de nodo virtual (`network.json`)

Si `LibreQoS.py` falla con `Virtual node promotion collision: 'AP_A' already exists at this level.`, hay un nodo con `"virtual": true` cuyos hijos colisionan por nombre al promoverse.

Renombre nodos en conflicto o reestructure jerarquía para evitar colisiones.
Para un visual del flujo lógico-a-físico y la asignación de CPU, consulte [Referencia avanzada de configuración](configuration-advanced-es.md).

### Se alcanzó el límite de circuitos mapeados

Si ve mensajes como:

- `Mapped circuit limit reached`
- `Bakery mapped circuit cap enforced`

LibreQoS está aplicando un límite de circuitos mapeados.

`ShapedDevices.csv` puede contener entradas ilimitadas, pero sin una suscripción/licencia Insight válida LibreQoS admite solo los primeros 1000 circuitos mapeados válidos al estado de shaping activo.

El límite predeterminado de 1000 circuitos mapeados aplica cuando Insight está:
- ausente
- expirado
- inválido por cualquier motivo
- operando con estado local de grant offline inválido

Síntomas típicos visibles para el operador:
- advertencia prominente de límite de circuitos mapeados en WebUI
- indicador de uso en la navegación izquierda mostrando cercanía o agotamiento del límite de 1000
- mensajes en `journalctl -u lqosd` con conteos requested/allowed/dropped
- shaping parcial, con circuitos fuera del límite activo quedando fuera del estado de shaping

Checks recomendados:

1. Confirmar estado de licencia Insight en UI.
2. Revisar logs de `lqosd` para requested/allowed/dropped.
3. Reducir circuitos mapeados (corto plazo) o ajustar licencia/límites (largo plazo).

### Códigos de problemas urgentes y primeras acciones

WebUI muestra códigos legibles por máquina para triage rápido.

| Código | Significado | Primeros checks | Ruta de corrección típica |
|---|---|---|---|
| `MAPPED_CIRCUIT_LIMIT` | Bakery está forzando límite de circuitos mapeados. | Estado de licencia Insight y `journalctl -u lqosd` con requested/allowed/dropped. | Reducir circuitos mapeados o actualizar licencia/límites. |
| `TC_U16_OVERFLOW` | IDs minor de clases/colas excedieron rango u16 de tc en una cola CPU. | `journalctl -u lqos_scheduler -u lqosd`, profundidad topológica y distribución por colas. | Aumentar paralelismo de colas y/o simplificar/rebalancear jerarquía. |
| `TC_QDISC_CAPACITY` | Los qdisc autoasignados planificados exceden el presupuesto seguro por interfaz o el preflight conservador de seguridad de memoria de Bakery antes de aplicar. | Conteos estimados por interfaz, desglose por tipo de qdisc y campos de memoria en el contexto del urgent issue, `journalctl -u lqos_scheduler -u lqosd`, configuración `on_a_stick` y `monitor_only`. | Reducir la carga planificada de qdisc para esta ejecución (por ejemplo menos circuitos/dispositivos en la forma de prueba) antes de reintentar; no confiar en una aplicación parcial. |
| `BAKERY_MEMORY_GUARD` | Un full reload chunked de Bakery fue detenido a mitad de aplicación porque la memoria disponible del host cayó por debajo del piso de seguridad. | `journalctl -u lqosd`, memoria disponible/total en el contexto del urgent issue y progreso reciente de aplicación de Bakery. | Tratar la ejecución como fallida, reducir presión de memoria o huella de colas y reintentar solo cuando el host esté estable. |
| `XDP_IP_MAPPING_CAPACITY` | Los mapeos IP requeridos exceden la capacidad actual del mapa XDP en el kernel. | Forma de `ShapedDevices.csv`, mezcla IPv4/IPv6, supuesto de un dispositivo frente a varios dispositivos, `journalctl -u lqos_scheduler -u lqosd`. | Reducir mapeos requeridos de inmediato (por ejemplo menos dispositivos o prueba IPv4-only), o aumentar la capacidad del mapa del kernel en un cambio coordinado. |
| `XDP_IP_MAPPING_APPLY_FAILED` | Uno o más inserts de mapeo IP fallaron durante la aplicación. | `journalctl -u lqos_scheduler -u lqosd` para ejemplos resumidos y conteos de fallo. | Corregir la causa del fallo y volver a ejecutar; no confiar en shaping parcial. |

Patrón operativo:

1. Abra el detalle del problema urgente en WebUI (código/mensaje/contexto).
2. Recolecte logs correlacionados de `lqosd` y `lqos_scheduler`.
3. Aplique mitigación inmediata.
4. Reconozca/limpie el evento en UI cuando esté estable.

## Páginas relacionadas

- [Quickstart](quickstart-es.md)
- [Configurar LibreQoS](configuration-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Ajuste de rendimiento](performance-tuning-es.md)
