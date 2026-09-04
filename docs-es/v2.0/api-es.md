# API del Nodo de LibreQoS

## Requisitos

A partir de LibreQoS 2.2, el servicio `lqos_api` usa la misma política de circuitos mapeados que LibreQoS:

- Las redes con 1.000 circuitos mapeados válidos o menos pueden usar la API sin una suscripción a Insight.
- Las redes con más de 1.000 circuitos mapeados válidos requieren un derecho válido de API o Insight.
- El conteo usa circuitos configurados únicos con mapeos válidos, no tráfico reciente. Varios dispositivos del mismo circuito cuentan una sola vez y las filas sin mapeo IP no cuentan.
- Una licencia activa o un grant firmado almacenado localmente puede autorizar el acceso a la API por encima del límite gratuito.

Es la misma población usada por el límite base de shaping:
- `ShapedDevices.csv` puede contener entradas ilimitadas.
- Sin una licencia o grant con derecho válido, LibreQoS admite solo los primeros 1.000 circuitos mapeados válidos al estado de shaping activo.
- Los conteos superiores requieren una licencia con derecho de API o Insight.

El acceso solo a la API está disponible mediante el enlace **Try Insight** de la WebUI a la mitad del precio de Insight completo.

## Fuente de verdad y pruebas

Use Swagger en su nodo como referencia completa y superficie de prueba para la versión instalada:

- Puerto local directo de la API: `http://<node-ip>:9122/api-docs`
- Si la WebUI usa HTTPS opcional con Caddy: `https://<hostname-o-ip-del-nodo>/api-docs/`

En sistemas que usan HTTPS con Caddy gestionado, deje el listener de la API en loopback y acceda a la API mediante `/api/v1/`. `LQOS_API_LISTEN` es un override avanzado para despliegues que exponen intencionalmente el listener de la API en forma directa; no lo configure en sistemas con Caddy gestionado salvo que la exposición directa, el firewall, TLS y los controles de acceso de operadores se hayan planificado juntos.

Use esta página como mapa de capacidades. Use Swagger para inventario completo de endpoints, esquemas request/response y pruebas en vivo.

¿Necesita definiciones de persistencia e impacto en runtime? Vea el [Glosario](glossary-es.md).

## Instalar y habilitar

Si instaló vía `.deb` (recomendado), `lqos_api` ya está en:

- `/opt/libreqos/src/bin/lqos_api`

Habilitar servicio:

```bash
sudo cp /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

La plantilla incluida de `lqos_api.service.example` espera `network-online.target`, para que la API no arranque antes de que DNS y la ruta por defecto esten listas.

Actualizar solo el binario de la API:

```bash
cd /opt/libreqos/src
./update_api.sh
```

Verificar estado del servicio:

```bash
sudo systemctl status lqos_api
```

## Autenticación

La mayoría de endpoints requieren:

- Header: `x-bearer`
- Valor: una clave local con nombre creada en **License & Services**, el token local heredado o su clave de licencia Insight/API-only

Las claves locales autentican al cliente, pero no evitan el límite de licenciamiento por circuitos mapeados. Un administrador puede crear hasta 16 claves con nombre en **License & Services**. LibreQoS muestra cada clave una sola vez, guarda únicamente su resumen SHA-256 y metadatos no secretos en `/etc/lqos.conf`, y no puede recuperarla después. Copie de inmediato el valor completo `lqos_api_...` al cliente y trátelo como una contraseña.

Las claves con nombre no caducan automáticamente. Revoque las que ya no necesite; la creación y la revocación pueden tardar hasta 30 segundos en surtir efecto en `lqos_api`. Las claves de licencia existentes y el antiguo `local_api.bearer_token` siguen siendo compatibles. La interfaz identifica este último como **Legacy local API key** y permite eliminarlo después de migrar los clientes.

## Qué pueden hacer los ISPs con la API

### 1) Ciclo de vida de suscriptores/circuitos

Aprovisionar, actualizar y eliminar registros de suscriptor/dispositivo.

- Alta o reemplazo:
  - `POST /overrides/persistent_devices`
  - `POST /shaped_devices/update`
- Ajustes de velocidad:
  - `POST /overrides/adjustments/circuit_speed`
  - `POST /overrides/adjustments/device_speed`
- Bajas:
  - `DELETE /overrides/persistent_devices/by_circuit/{circuit_id}`
  - `DELETE /overrides/persistent_devices/by_device/{device_id}`
  - `POST /overrides/adjustments/remove_circuit`
  - `POST /overrides/adjustments/remove_device`

### 2) Gestión de overrides y políticas

Mantener políticas persistentes para circuitos, dispositivos, sitios y overrides específicos de UISP.

- `GET/POST/DELETE /overrides/adjustments*`
- `GET/POST/DELETE /overrides/network_adjustments*`
- `GET/POST/DELETE /overrides/uisp/bandwidth*`
- `GET/POST/DELETE /overrides/uisp/routes*`

### 3) Topología y archivos de entrada de shaping

Inspeccionar y actualizar archivos de topología/shaping usados por los flujos de runtime.

- `GET /network_json/json`
- `GET /network_json/text`
- `POST /network_json/update`
- `POST /network_json/set_site_speed`
- `POST /network_json/set_site_speed_batch`
- `GET /shaped_devices`
- `POST /shaped_devices/update`

### 4) Monitoreo y diagnóstico

Leer estado de salud, scheduler, throughput, flujos, colas y circuitos.

Endpoints representativos:
- `GET /health`
- `GET /status_snapshot`
- `GET /scheduler_status`
- `GET /circuit/{circuit_id}`
- `GET /search`
- `GET /current_throughput`
- `GET /queue_stats_total`
- `GET /warnings`
- `GET /urgent`, `GET /urgent/status`

### 5) Operaciones de control y reload

Disparar acciones de control operativo cuando sea necesario.

- `POST /reload_libreqos`
- `POST /clear_hot_cache`

Trátelas como operaciones de mayor riesgo en producción.

## Modelo de persistencia (importante)

No todas las escrituras persisten del mismo modo:

- `overrides/*`: estado de política persistente.
- `network_json/set_site_speed*`: ediciones transitorias.
- `network_json/update` y `shaped_devices/update`: reemplazo directo de archivos, generalmente sobrescribible por integración.

Si hay integraciones habilitadas, sus ciclos de refresco pueden sobrescribir ediciones directas de archivos.

## Flujo recomendado para producción

1. Validar comportamiento del endpoint y esquema de payload en Swagger.
2. Aplicar el cambio mínimo necesario.
3. Verificar resultado con checks de solo lectura (`/health`, `/scheduler_status`, vistas de circuito/throughput).
4. Mantener snapshots de rollback para operaciones que cambien config/datos.

## Hardening de despliegue

- Limite acceso de API a redes de gestión confiables.
- No exponga la API directamente a Internet público.
- Si necesita acceso remoto, use reverse proxy con TLS y autenticación.
- Restrinja acceso entrante con allowlists de firewall.

Si quiere que la WebUI y la documentación de la API compartan el mismo origen HTTPS, vea [HTTPS opcional con Caddy](https-caddy-es.md).
