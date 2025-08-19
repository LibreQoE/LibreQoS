# Demonios de LibreQoS 

lqosd

- Gestiona el código XDP. Construido en Rust.

lqos_node_manager

- Ejecuta la GUI disponible en http://a.b.c.d:9123

lqos_scheduler

- lqos_scheduler maneja estadísticas y realiza actualizaciones continuas de los modeladores de LibreQoS, incluida la extracción de cualquier integración de CRM habilitada (UISP, Splynx).
- Al iniciar: ejecuta una configuración completa de colas
- Cada 10 segundos: Grafica estadísticas de ancho de banda y latencia
- Cada 30 segundos: Actualiza colas, extrayendo nueva configuración de la integración de CRM si está habilitada

## Ejecutar daemons con systemd

Puede configurar `lqosd`, `lqos_node_manager`, y `lqos_scheduler` como servicios systemd.

```shell
sudo cp /opt/libreqos/src/bin/lqos_node_manager.service.example /etc/systemd/system/lqos_node_manager.service
sudo cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
sudo cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
```

Finalmente, correr

```shell
sudo systemctl daemon-reload
sudo systemctl enable lqosd lqos_node_manager lqos_scheduler
```

Ahora puede apuntar un navegador web a `http://a.b.c.d:9123` (reemplazar `a.b.c.d` con la dirección IP de administración de su servidor de modelado) y disfrute de una vista en tiempo real de su red.

## Depuración lqos_scheduler

En el fondo, lqos_scheduler corre scheduler.py, que a su vez corre LibreQoS.py

Las ejecuciones únicas de estos componentes individuales pueden ser muy útiles para la depuración y para asegurarse de que todo esté configurado correctamente.

Primero, detener lqos_scheduler

```shell
sudo systemctl stop lqos_scheduler
```

Para ejecuciones iniciales de of LibreQoS.py, utilice

```shell
sudo ./LibreQoS.py
```

- Para utilizar el modo de depuración con una salida más detallada, utilice:

```shell
sudo ./LibreQoS.py --debug
```

Para confirmar que lqos_scheduler (scheduler.py) puede funcionar correctamente, ejecute:

```shell
sudo python3 scheduler.py
```

Una vez que haya eliminado todos los errores, reinicie lqos_scheduler con

```shell
sudo systemctl start lqos_scheduler
```
