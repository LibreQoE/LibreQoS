# API del Nodo de LibreQoS

## Requisitos
La API de nodo LibreQoS requiere una suscripción activa a LibreQoS Insight.

## Instalación

### Descarga

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://libreqos.io/wp-content/uploads/2025/09/lqos_api.zip
sudo apt-get install unzip
sudo mkdir /opt/lqos_api/
sudo chown -R $USER /opt/lqos_api/
cd /opt/lqos_api/
unzip lqos_api.zip
```

### Ejecución de prueba
```
cd /opt/lqos_api/
./lqos_api
```

### Servicio systemd

Si la ejecución de prueba funciona correctamente, use `sudo nano /etc/systemd/system/lqos_api.service` y pegue el siguiente contenido:

```
[Unit]
After=network.service lqosd.service
Requires=lqosd.service

[Service]
WorkingDirectory=/opt/lqos_api/
ExecStart=/opt/lqos_api/lqos_api
Restart=always

[Install]
WantedBy=default.target
```
Luego ejecute:
```
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

## Uso

Vea `localhost:9122/api-docs` para la interfaz de Swagger UI.

## Endpoints de la API

### Salud y sistema
- `/health` - Comprobación de salud del servicio
- `/reload_libreqos` - Recargar la configuración de LibreQoS
- `/validate_shaped_devices` - Validar el CSV de dispositivos modelados
- `/lqos_stats` - Obtener estadísticas internas de lqosd
- `/stormguard_stats` - Obtener estadísticas de Stormguard
- `/bakery_stats` - Obtener el recuento de circuitos activos de Bakery

### Métricas de tráfico
- `/current_throughput` - Estadísticas actuales de rendimiento de la red
- `/top_n_downloaders/{start}/{end}` - Descargadores principales por tráfico
- `/worst_rtt/{start}/{end}` - Hosts con peor tiempo de ida y vuelta (RTT)
- `/worst_retransmits/{start}/{end}` - Hosts con más retransmisiones TCP
- `/best_rtt/{start}/{end}` - Hosts con mejor tiempo de ida y vuelta (RTT)
- `/host_counter` - Todos los contadores de tráfico por host

### Topología de red
- `/network_map/{parent}` - Mapa de red desde el nodo padre
- `/full_network_map` - Topología de red completa
- `/top_map_queues/{n}` - Las N colas principales por tráfico
- `/node_names` - Obtener nombres de nodos a partir de IDs
- `/funnel/{target}` - Analizar el embudo de tráfico hacia el circuito

### Gestión de circuitos
- `/all_circuits` - Listar todos los circuitos
- `/raw_queue_data/{circuit_id}` - Datos en bruto de colas para el circuito

### Análisis de flujos
- `/dump_active_flows` - Volcar todos los flujos activos (lento)
- `/count_active_flows` - Conteo de flujos activos
- `/top_flows/{flow_type}/{n}` - Flujos principales por tipo de métrica
- `/flows_by_ip/{ip}` - Flujos para una IP específica
- `/flow_duration` - Estadísticas de duración de flujos

### Geo y protocolos
- `/current_endpoints_by_country` - Endpoints de tráfico por país
- `/current_endpoint_latlon` - Coordenadas geográficas de endpoints
- `/ether_protocol_summary` - Resumen de protocolos Ethernet
- `/ip_protocol_summary` - Resumen de protocolos IP
