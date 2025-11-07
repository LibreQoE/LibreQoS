# Componentes de Software de LibreQoS

## Servicios de Systemd
### lqosd

- Administra el código XDP actual.
- Desarrollado en Rust.
- Ejecuta la interfaz gráfica (GUI) disponible en http://a.b.c.d:9123

### lqos_scheduler

- lqos_scheduler realiza actualizaciones continuas de los reguladores de tráfico (shapers) de LibreQoS, incluyendo la obtención de datos desde cualquier integración CRM habilitada (UISP, Splynx, Netzur).
- Acciones realizadas:
  - Al iniciar: Realiza una configuración completa de las filas.
  - Cada X minutos: Actualiza las colas, obteniendo nueva configuración desde la integración CRM, si está habilitada.
    - The default minute interval is 30, so the refresh occurs every 30 minutes by default.
    - El intervalo de minutos por defecto es de 30, por lo tanto, la actualización se realiza cada 30 minutos.
    - El intervalo de minutos puede ajustarse mediante el parámetro `queue_refresh_interval_mins` ubicado en `/etc/lqos.conf`.

### Verificar el estado de los servicios

```
sudo systemctl status lqosd lqos_scheduler
```
Si el estado de alguno de los dos servicios aparece como 'failed', debe examinarse la causa utilizando journalctl, el cual muestra el historial completo del servicio. Por ejemplo, si lqosd ha fallado, ejecute:
```
sudo journalctl -u lqosd -b
```
Presione la tecla End en su teclado para ir al final del registro y ver las actualizaciones más recientes.

Lqosd indicará las razones específicas por las cuales falló, como una interfaz que no está activa, falta de soporte para múltiples filas en la interfaz, entre otros problemas.

### Depuración de lqos_scheduler

En segundo plano, lqos_scheduler ejecuta el script de Python scheduler.py, el cual a su vez ejecuta el script de Python LibreQoS.py

- scheduler.py: realiza actualizaciones continuas de los reguladores de tráfico (shapers) de LibreQoS, incluyendo la obtención de datos desde cualquier integración CRM habilitada (UISP, Splynx, Netzur).
- LibreQoS.py: se encarga de crear y actualizar las filas y el regulamiento de tráfico de los dispositivos.

Ejecuciones puntuales de estos componentes individuales pueden ser de gran ayuda para depurar y asegurar que todo esté correctamente configurado.

Primero, detenga el servicio lqos_scheduler:

```shell
sudo systemctl stop lqos_scheduler
```

Para realizar una ejecución única de LibreQoS.py, utilice:

```shell
sudo ./LibreQoS.py
```

- Para ejecutar en modo depuración con respuestas mas detalladas, use:

```shell
sudo ./LibreQoS.py --debug
```

Para confirmar que lqos_scheduler (scheduler.py) funciona correctamente, ejecute:

```shell
sudo python3 scheduler.py
```

Una vez que se hayan corregido los errores, reinicie el servicio lqos_scheduler con:

```shell
sudo systemctl start lqos_scheduler
```
