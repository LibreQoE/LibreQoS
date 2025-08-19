# Solución de Problemas

## Problemas Comunes

### LibreQoS se está ejecutando, pero no está regulando el tráfico

En ispConfig.py, asegúrese de que las interfaces edge y core correspondan correctamente a la interfaz de borde (edge) y núcleo (core). Pruebe intercambiando las interfaces para ver si el tráfico empieza a regularse correctamente.

Asegúrese de que sus servicios se estén ejecutando correctamente:

- `lqosd.service`
- `lqos_node_manager`
- `lqos_scheduler`

Node manager y scheduler dependen de que el servicio `lqos.service` esté en buen estado y en ejecución.

Por ejemplo, para verificar el estado de lqosd, ejecute:
```sudo systemctl status lqosd```

### lqosd no se está ejecutando o falló al iniciar
En la terminal, ejecute ```sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd```. Esto proporcionará detalles sobre por qué falló al iniciar.

### RTNETLINK answers: Invalid argument

Este error suele aparecer cuando el "MQ qdisc" no puede agregarse correctamente a la interfaz NIC. Esto sugiere que la NIC tiene un número insuficiente de filas RX/TX. Por favor asegúrese de estar utilizando las [NICs recomendadas](../SystemRequirements/Networking.md).

### InfluxDB "Failed to update bandwidth graphs"

El scheduler (scheduler.py) ejecuta la integración con InfluxDB dentro de una instrucción try/except.
Si falla al actualizar InfluxDB, mostrará "Failed to update bandwidth graphs".
Para encontrar la causa exacta del error, ejecute: ```python3 graphInfluxDB.py``` lo cual mostrará errores más específicos.

### Todas las IPs de clientes aparecen bajo Unknown IPs en lugar de Shaped Devices en la Interfaz Gráfica
```
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo python3 LibreQoS.py
```

La salida de consola al ejecutar LibreQoS.py directamente proporciona errores más específicos relacionados con ShapedDevices.csv y network.json. Una vez que haya identificado y corregido el error en ShapedDevices.csv y/o network.json, ejecute:

```sudo systemctl start lqos_scheduler```
