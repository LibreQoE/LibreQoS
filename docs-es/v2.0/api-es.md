# API del Nodo de LibreQoS

## Requisitos

La API del nodo LibreQoS requiere una suscripción activa a LibreQoS Insight.

## Instalación

### Si instaló vía `.deb` (recomendado)

`lqos_api` se instala con LibreQoS en:

`/opt/libreqos/src/bin/lqos_api`

### Ejecución de prueba

```bash
/opt/libreqos/src/bin/lqos_api
```

### Servicio systemd

El paquete incluye un archivo de servicio de ejemplo:

```bash
sudo cp /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
```

Luego ejecute:

```bash
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

### Actualizar solo el binario de la API

Use el script de ayuda:

```bash
cd /opt/libreqos/src
./update_api.sh
```

Esto descarga el `lqos_api` más reciente e instala en `/opt/libreqos/src/bin/lqos_api`.

Puede omitir el reinicio del servicio:

```bash
./update_api.sh --no-restart
```

O instalar en otra ruta:

```bash
./update_api.sh --bin-dir /alguna/ruta/bin --no-restart
```

### Archivo de servicio manual (si es necesario)

Si crea el servicio manualmente, use:

```
[Unit]
After=network.service lqosd.service
Requires=lqosd.service

[Service]
WorkingDirectory=/opt/libreqos/src/bin
ExecStart=/opt/libreqos/src/bin/lqos_api
Restart=always

[Install]
WantedBy=default.target
```

Luego ejecute:

```bash
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

## Uso

Vea `localhost:9122/api-docs` para la interfaz Swagger.

`/api-docs` es la fuente de verdad para su versión instalada. La disponibilidad de endpoints puede cambiar entre versiones.

## Guía de despliegue y hardening

Postura recomendada para producción:

1. Mantenga la API accesible solo desde redes de gestión confiables.
2. No exponga la API directamente a Internet público.
3. Si requiere acceso remoto, publíquela detrás de un reverse proxy con TLS y autenticación.
4. Restrinja acceso entrante con allowlists de firewall (host/red).
5. Monitoree logs de API/servicio para detectar accesos anómalos.

Si la API no se necesita en un nodo, deshabilite el servicio:

```bash
sudo systemctl disable --now lqos_api
```

Si está habilitada, verifique el estado tras actualizaciones:

```bash
sudo systemctl status lqos_api
```

## Endpoints de la API

### Salud y sistema
- `/health` - Verificación de salud del servicio
- `/reload_libreqos` - Recarga configuración de LibreQoS
- `/validate_shaped_devices` - Valida CSV de Shaped Devices
- `/lqos_stats` - Estadísticas internas de lqosd
- `/stormguard_stats` - Estadísticas de StormGuard
- `/bakery_stats` - Conteo de circuitos activos de Bakery

### Métricas de tráfico
- `/current_throughput` - Throughput actual de red
- `/top_n_downloaders/{start}/{end}` - Top descargadores por tráfico
- `/worst_rtt/{start}/{end}` - Hosts con peor RTT
- `/worst_retransmits/{start}/{end}` - Hosts con más retransmisiones TCP
- `/best_rtt/{start}/{end}` - Hosts con mejor RTT
- `/host_counter` - Contadores de tráfico por host

### Topología de red
- `/network_map/{parent}` - Mapa de red desde nodo padre
- `/full_network_map` - Topología completa
- `/top_map_queues/{n}` - Top N colas por tráfico
- `/node_names` - Nombres de nodos por ID
- `/funnel/{target}` - Análisis de embudo hacia circuito

### Gestión de circuitos
- `/all_circuits` - Lista de todos los circuitos
- `/raw_queue_data/{circuit_id}` - Datos de cola en bruto por circuito

### Análisis de flujos
- `/dump_active_flows` - Volcado de flujos activos (lento)
- `/count_active_flows` - Conteo de flujos activos
- `/top_flows/{flow_type}/{n}` - Top flujos por tipo de métrica
- `/flows_by_ip/{ip}` - Flujos por IP específica
- `/flow_duration` - Estadísticas de duración de flujos

### Geo y protocolos
- `/current_endpoints_by_country` - Endpoints de tráfico por país
- `/current_endpoint_latlon` - Coordenadas geográficas de endpoints
- `/ether_protocol_summary` - Resumen de protocolos Ethernet
- `/ip_protocol_summary` - Resumen de protocolos IP

## Cobertura de API en versiones recientes

Compilaciones recientes ampliaron cobertura en algunas áreas. Dependiendo de su versión, la API también puede incluir:

- Resúmenes de alertas y warnings
- Resúmenes de conteo de dispositivos y circuitos
- Consulta de detalle para circuito individual
- Helpers de búsqueda para flujos de UI
- Endpoints de detalle de scheduler y estadísticas de colas

Use `localhost:9122/api-docs` para confirmar rutas y esquemas exactos en su nodo.

## Capturar inventario de endpoints por release

Para notas de release o runbooks internos, capture un snapshot de rutas API desde el nodo:

```bash
curl -s http://localhost:9122/api-docs | jq -r '.paths | keys[]' | sort
```

Guarde esta salida junto con artefactos del release para comparar diferencias entre versiones.
