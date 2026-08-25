# Solución de problemas

## Empiece aquí: triage por síntoma

Use esta tabla para ir al primer check rápidamente.

¿Necesita definiciones de términos de licencia/scheduler? Vea el [Glosario](glossary-es.md).

| Síntoma | Primer check | Ubicación en WebUI | Siguiente sección |
|---|---|---|---|
| No se puede acceder a la WebUI | `systemctl status lqosd caddy` | N/A (UI no disponible) | No hay WebUI en x.x.x.x:9123 o en la URL HTTPS |
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

Elimine `lqusers.toml` solo si desea reiniciar el acceso o si el archivo está corrupto y no se puede reparar. Después de eliminarlo, reinicie `lqosd` y abra `IP_CAJA:9123/login.html` si SSL está deshabilitado; la WebUI debería redirigirlo automáticamente al flujo de primer inicio.

### No hay WebUI en x.x.x.x:9123 o en la URL HTTPS

La WebUI depende de `lqosd`. Si HTTPS opcional con Caddy está habilitado, `caddy` también debe estar saludable.

```bash
sudo systemctl status lqosd caddy
```

Luego:

- Si SSL está deshabilitado, pruebe `http://tu-ip-de-gestión:9123/`
- Si SSL está habilitado con hostname, pruebe `https://tu-hostname/`
- Si SSL está habilitado sin hostname, pruebe `https://tu-ip-de-gestión/`
- Si el navegador advierte en modo de certificado local, confíe en `/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt` en la estación de trabajo del operador

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

Si el log muestra `LibreQoS failed to attach the XDP/TC kernel` o `Unable to load the XDP/TC kernel`, trate el arranque de `lqosd` como fallido. La WebUI y el bus local no arrancan hasta que el programa del kernel se cargue y se adjunte correctamente. El error de carga incluye el valor de retorno bruto, el número errno y el código errno, por ejemplo `raw=-11, errno=11, code=EAGAIN`. Revise si hay un programa XDP existente, un hook TC ocupado, falta de soporte del driver o mapas BPF fijados obsoletos antes de reiniciar `lqosd`.

### Uso de CPU elevado en horas pico (clocksource TSC inestable)

Si la CPU se satura al 100% durante las horas pico y el throughput/latencia se degradan, el kernel de Linux puede haber marcado el TSC de la CPU como un clocksource inestable y haber vuelto a uno más lento (normalmente HPET). Los operadores de LibreQoS han observado este comportamiento en algunos hosts AMD Ryzen con los estados C de la CPU habilitados, a veces solo después de reiniciar.

LibreQoS advierte al arrancar cuando HPET o el temporizador PM de ACPI está activo. Si ve esta advertencia de lqosd, compruebe el clocksource directamente:

```bash
cat /sys/devices/system/clocksource/clocksource0/current_clocksource
```

Si la salida es `hpet` o `acpi_pm` en un host físico de LibreQoS, soluciónelo de una de estas formas:

- añadiendo `tsc=reliable` (o `tsc=nowatchdog`) a la línea de comandos del kernel en `/etc/default/grub` (`GRUB_CMDLINE_LINUX_DEFAULT`) y ejecutando `sudo update-grub`, y luego reiniciando, o
- deshabilitando los estados C de la CPU en el BIOS/UEFI.

En los hosts donde el watchdog de estabilidad degrada el TSC, añadir únicamente `clocksource=tsc` no evita esa degradación.

### Depuración avanzada de lqosd

```bash
sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd
```

### El servicio lqos_scheduler muestra errores

```bash
sudo journalctl -u lqos_scheduler --since "1 day ago" --no-pager > lqos_sched_log.txt
```

Las instalaciones empaquetadas mantienen las dependencias Python de LibreQoS en `/opt/libreqos/venv`. Los servicios siguen ejecutándose como root, pero los paquetes Python no se mezclan con los paquetes administrados por apt. Si el scheduler informa módulos faltantes, o si la configuración del paquete se interrumpió al instalar dependencias Python, reconstruya el entorno virtual:

```bash
sudo /opt/libreqos/src/bin/rebuild_python_venv.sh
sudo dpkg --configure -a
sudo systemctl restart lqosd lqos_scheduler
```

Las instalaciones basadas en git deben usar `./build_rust.sh` después de actualizar. Ese script reconstruye el entorno virtual antes de actualizar los archivos de servicio o reiniciar servicios. Si systemd informa `status=203/EXEC` en `/opt/libreqos/venv/bin/python`, o una falla en la comprobación previa del scheduler, reconstruya el entorno virtual con el comando anterior y reinicie `lqos_scheduler`.

Las instalaciones antiguas anteriores al entorno virtual pueden mostrar `ModuleNotFoundError` y recomendar comandos de `pip` del sistema. No repare instalaciones actuales con `pip` del sistema ni con `--break-system-packages`; esos paquetes no los usa el servicio `lqos_scheduler` respaldado por el entorno virtual. Actualice a un paquete que cree `/opt/libreqos/venv` y use el comando de reparación anterior.

Si el scheduler falla inmediatamente después de un reinicio con un mensaje como `Socket (typically /run/lqos/bus) not found`, eso indica que `lqosd` todavía no había terminado de enlazar el bus local. Los builds actuales esperan brevemente la disponibilidad del bus al arrancar el scheduler en lugar de abortar de inmediato, por lo que ya no deberían aparecer panics repetidos de arranque tras un reinicio.

Si `journalctl -u lqosd` muestra `lqosd host memory pressure` o `lqosd process memory critical`, el daemon detectó uso alto de memoria y registró contexto de diagnóstico. El watchdog no reinicia `lqosd`; registra memoria disponible, memoria total, RSS/swap de `lqosd`, cantidad de hilos, cantidad de flujos y contadores de tiempo que ayudan a diagnosticar el origen del crecimiento de memoria. La presión de memoria del host se registra cuando la memoria disponible está por debajo del 10% de la RAM instalada. La memoria del proceso se registra como crítica cuando el RSS más swap de `lqosd` alcanza el 90% de la RAM instalada.

Puede desactivar estos diagnósticos con un override de entorno de systemd durante ventanas cortas de diagnóstico:

```bash
sudo systemctl edit lqosd
```

Use `LQOSD_MEMORY_WATCHDOG_DISABLED=1` solamente cuando esté observando activamente la presión de memoria con otra herramienta.

### El estado del scheduler en WebUI aparece no saludable

Versiones recientes muestran estado/readiness del scheduler en WebUI.
Si el modal del scheduler indica que se agotó el tiempo al cargar detalles, las versiones actuales mantienen visible la última instantánea buena del scheduler con su antigüedad en lugar de convertir ese problema de transporte en un error del scheduler. Trate primero esa advertencia como un problema de comunicación entre WebUI y `lqosd`, y confirme la salud real del scheduler en los logs antes de asumir que falló el shaping.
Si falla un subproceso de integración pero el shaping puede continuar con la última topología válida, el scheduler puede seguir mostrando estado ready, pero el último error de integración permanece visible en el estado del scheduler hasta la siguiente integración exitosa.
Si el scheduler no puede leer `lqos_overrides.json` o sus capas materializadas porque otro proceso mantiene el lock de overrides, las versiones actuales reintentan brevemente y luego bloquean esa recarga. La topología anterior permanece en uso, y el error del scheduler incluye detalles del proceso que mantiene el lock, como PID, nombre de proceso, operación y hora de creación cuando estén disponibles.

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

Si el shaping de arranque comienza antes de que topology runtime publique la generación actual de `shaping_inputs.json`, las versiones actuales mantienen el scheduler en un estado de espera de arranque y reintentan el shaping inicial cada pocos segundos. Un mensaje breve como `still building outputs for the current source generation` justo después de reiniciar normalmente significa que LibreQoS todavía está terminando el ciclo de importación y publicación de runtime, no que el shaping quede detenido hasta la siguiente actualización de 30 minutos.

Si una actualización programada de integración cae mientras topology runtime todavía está publicando salidas para la nueva generación de fuente, las versiones actuales mantienen el scheduler en estado de espera para esa generación y reintentan automáticamente el shaping programado en cuanto topology runtime termina. Trate `Scheduled shaping refresh deferred` como una espera transitoria solo cuando el mensaje indique que topology runtime todavía está construyendo salidas para la generación actual. Si en cambio indica que topology runtime falló para la generación actual, revise directamente esa falla de runtime.

Si el arranque del scheduler permanece demasiado tiempo en esa espera, o entra en estado degradado con un mensaje indicando que topology runtime falló para la generación actual, revise:

```bash
cat /opt/libreqos/state/topology/topology_runtime_status.json
ls -lh /opt/libreqos/state/topology/topology_effective_state.json /opt/libreqos/state/topology/network.effective.json /opt/libreqos/state/shaping/shaping_inputs.json
journalctl -u lqos_scheduler --since "30 minutes ago"
journalctl -u lqosd --since "30 minutes ago"
```

Si `journalctl -u lqosd` muestra advertencias repetidas como `BeginIngest queue full`, `IngestChunk queue full` o `EndIngest queue full` durante el arranque o justo después de una importación de topología, los builds anteriores estaban descartando tramas de ingesta hacia Insight porque la cola local del canal de control era demasiado pequeña para ráfagas grandes. Los builds actuales aplican backpressure sobre el socket de Insight para esos lotes de ingesta, por lo que esas advertencias ya no deberían aparecer durante ráfagas cortas de arranque o importación. Si siguen apareciendo después de actualizar, revise primero la presión de CPU de `lqosd` y la conectividad reciente del canal de control antes de asumir que el shaping está fallando.

### RTNETLINK answers: Invalid argument

Suele indicar que no se pudo agregar correctamente qdisc MQ en la NIC (colas RX/TX insuficientes). Verifique [NICs recomendadas](requirements-es.md).

### Todas las IPs de clientes aparecen como Unknown IPs

```bash
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo /opt/libreqos/venv/bin/python /opt/libreqos/src/LibreQoS.py
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

### Los circuitos muestran Generated_PN en vez de nombres de network.json

Si un despliegue DIY/manual usa `network.json` y `ShapedDevices.csv`, pero la página de Circuitos muestra padres como `Generated_PN_1`, revise las entradas de shaping runtime:

```bash
jq '.circuits[] | {circuit_id, logical_parent_node_name, effective_parent_node_name, resolution_source}' /opt/libreqos/state/shaping/shaping_inputs.json
```

Si `resolution_source` es `flat_bucket`, LibreQoS está en modo de topología flat. Ese modo asigna intencionalmente los circuitos a colas generadas por CPU en vez de moldearlos bajo la jerarquía nombrada por `Parent Node`.

Para moldear circuitos bajo los nombres de nodo de `network.json`, configure:

```toml
[topology]
compile_mode = "full"
```

Después recargue o espere al scheduler para regenerar `network.effective.json` y `shaping_inputs.json`.

### Colisión de promoción de nodo virtual (`network.json`)

Si `LibreQoS.py` falla con `Virtual node promotion collision: 'AP_A' already exists at this level.`, hay un nodo con `"virtual": true` cuyos hijos colisionan por nombre al promoverse.

Renombre nodos en conflicto o reestructure jerarquía para evitar colisiones.
Para un visual del flujo lógico-a-físico y la asignación de CPU, consulte [Referencia avanzada de configuración](configuration-advanced-es.md).

### Se alcanzó el límite de circuitos mapeados

Si ve mensajes como:

- `Mapped circuit limit reached`
- `Bakery mapped circuit cap enforced`

LibreQoS está aplicando un límite de circuitos mapeados.

`ShapedDevices.csv` puede contener entradas ilimitadas, pero sin un estado válido de licencia/grant de Insight o Local LibreQoS admite solo los primeros 1000 circuitos mapeados válidos al estado de shaping activo.

El límite predeterminado de 1000 circuitos mapeados aplica cuando el estado de licencia/grant está:
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

1. Confirmar el estado de licencia en la página `License & Services`.
2. Revisar logs de `lqosd` para requested/allowed/dropped.
3. Reducir circuitos mapeados (corto plazo) o ajustar licencia/límites (largo plazo).

La API del nodo usa el mismo conteo a partir de LibreQoS 2.2. Con 1.000 circuitos mapeados o menos, confirme que haya una clave local con nombre (o una clave heredada/de licencia compatible) y que `lqos_api` esté en ejecución. Una clave nueva o revocada puede tardar hasta 30 segundos en surtir efecto. Por encima de 1.000, confirme además que el derecho efectivo de API o Insight sea válido.

### Códigos de problemas urgentes y primeras acciones

WebUI muestra códigos legibles por máquina para triage rápido.

| Código | Significado | Primeros checks | Ruta de corrección típica |
|---|---|---|---|
| `MAPPED_CIRCUIT_LIMIT` | Bakery está forzando límite de circuitos mapeados. | Estado de licencia Insight y `journalctl -u lqosd` con requested/allowed/dropped. | Reducir circuitos mapeados o actualizar licencia/límites. |
| `TC_U16_OVERFLOW` | IDs minor de clases/colas excedieron rango u16 de tc en una cola CPU. | `journalctl -u lqos_scheduler -u lqosd`, profundidad topológica y distribución por colas. | Aumentar paralelismo de colas y/o simplificar/rebalancear jerarquía. |
| `TC_QDISC_CAPACITY` | Los qdisc autoasignados planificados exceden el presupuesto seguro por interfaz o el preflight conservador de seguridad de memoria de Bakery antes de aplicar. | Conteos estimados por interfaz, desglose por tipo de qdisc y campos de memoria en el contexto del urgent issue, `journalctl -u lqos_scheduler -u lqosd`, configuración `on_a_stick` y `monitor_only`. | Reducir la carga planificada de qdisc para esta ejecución (por ejemplo menos circuitos/dispositivos en la forma de prueba) antes de reintentar; no confiar en una aplicación parcial. |
| `BAKERY_MEMORY_GUARD` | Una recarga completa por fragmentos de Bakery fue detenida a mitad de aplicación porque la memoria disponible del host cayó por debajo del piso de seguridad escalado. | `journalctl -u lqosd`, memoria disponible/total en el contexto del urgent issue y progreso reciente de aplicación de Bakery. | Tratar la ejecución como fallida, reducir presión de memoria o huella de colas y reintentar solo cuando el host esté estable. |
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
