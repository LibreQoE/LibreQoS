# API del Nodo de LibreQoS

> **Documenta la API que se entrega con v3.** La API forma parte de `lqosd`: no es
> un servicio ni un paquete aparte. El resto del producto se documenta en el árbol
> de documentación de v3 (en inglés).

## Qué es

`lqosd` enlaza la API como biblioteca y la sirve en `/api/v3` sobre el mismo
listener del gestor del nodo (por defecto `:9123`). No hay nada extra que
instalar, habilitar, descargar ni actualizar:

- Ruta base de la API: `/api/v3`
- Documentación interactiva (Swagger UI): `/api/v3/docs`
- Especificación legible por máquinas: `/api/v3/openapi.json`
- Dirección de escucha, TLS y listener: los del gestor del nodo (vea la guía
  «operate» de v3)

Como la API comparte el listener del gestor del nodo, exponer el gestor expone la
API. Mantenga ese listener en redes de gestión de confianza.

## Autenticación

Toda petición necesita una clave de API local en la cabecera `x-bearer`:

```bash
curl -sS -H "x-bearer: $LQOS_API_KEY" http://<ip-del-nodo>:9123/api/v3/health
```

Cree las claves en la WebUI, en **License & Services → Local API Keys**. Un
administrador puede generar hasta 16 claves con nombre. LibreQoS muestra cada
clave una sola vez, guarda únicamente su digest SHA-256 y metadatos no secretos en
`/etc/lqos.conf`, y no puede recuperarla después: copie el valor completo
`lqos_api_...` al cliente de inmediato y trátelo como una contraseña.

Las claves con nombre no expiran automáticamente; revoque las que ya no necesite.
El verificador lee la configuración vigente en cada petición, así que crear o
revocar una clave surte efecto de inmediato, sin reiniciar. El antiguo
`local_api.bearer_token` sigue funcionando y la UI lo muestra como **Legacy local
API key**; elimínelo cuando sus clientes usen claves con nombre.

No hay requisito de licencia, ni límite de cantidad de circuitos, ni suscripción
que condicione el acceso a la API: una clave válida es todo el requisito.

## Fuente de verdad y pruebas

Swagger en su propio nodo es la referencia completa y la superficie de prueba de
la versión que está ejecutando:

- Directo: `http://<ip-del-nodo>:9123/api/v3/docs`
- Detrás del proxy HTTPS opcional de Caddy (que hace proxy del gestor del nodo):
  `https://<hostname>/api/v3/docs`

Use esta página como mapa de capacidades. Use Swagger para el inventario completo
de endpoints, los esquemas request/response y las pruebas en vivo.

¿Necesita definiciones de persistencia e impacto en runtime? Vea el
[Glosario](glossary-es.md).

## Qué puede hacer un ISP con la API

Todas las rutas son relativas a `/api/v3`.

### 1) Ciclo de vida de suscriptores y circuitos

Aprovisione, actualice y elimine registros de suscriptores y dispositivos.

- Agregar o reemplazar:
  - `POST /overrides/persistent_devices`
  - `POST /shaped_devices/update`
- Ajustar velocidades:
  - `POST /overrides/adjustments/circuit_speed`
  - `POST /overrides/adjustments/device_speed`
- Eliminar:
  - `DELETE /overrides/persistent_devices/by_circuit/{circuit_id}`
  - `DELETE /overrides/persistent_devices/by_device/{device_id}`
  - `POST /overrides/adjustments/remove_circuit`
  - `POST /overrides/adjustments/remove_device`

### 2) Gestión de overrides y políticas

Mantenga políticas persistentes de overrides para circuitos, dispositivos, sitios y
overrides específicos de UISP.

- `GET/POST/DELETE /overrides/adjustments*`
- `GET/POST/DELETE /overrides/network_adjustments*`
- `GET/POST/DELETE /overrides/uisp/bandwidth*`
- `GET/POST/DELETE /overrides/uisp/routes*`

### 3) Topología y archivos de entrada del shaping

Inspeccione y actualice los archivos de shaping/topología que usan los flujos de
trabajo en ejecución.

- `GET /network_json/json`
- `GET /network_json/text`
- `GET /shaped_devices`
- `POST /shaped_devices/update`

`POST /network_json/set_site_speed` y `POST /network_json/set_site_speed_batch`
se eliminaron en v3: responden un `404` JSON con clave (`401` sin clave) y no
forman parte del montaje. Use `POST /overrides/network_adjustments/site_speed`
(solo ancho máximo). El manual de la API de v3
(`src/src/rust/lqos_api/manual/`, generado en `/api/v3/openapi.json`) es la
referencia única de endpoints; esta página es el resumen para el operador.

Las escrituras de topología antiguas (`/network_json/update`, `/network_json/node`,
`/network/reparent_site`) están retiradas: responden `409` y remiten al Gestor de
Topología de LibreQoS. Los cambios de reparentado y de velocidad de sitio
corresponden a los endpoints de overrides.

### 4) Monitoreo y diagnóstico

Lea salud, planificador, throughput, flujos, colas y estado de circuitos.

Endpoints representativos:
- `GET /health`
- `GET /version`
- `GET /status_snapshot`
- `GET /scheduler_status`
- `GET /circuit/{circuit_id}`
- `GET /search`
- `GET /current_throughput`
- `GET /queue_stats_total`
- `GET /warnings`
- `GET /urgent`, `GET /urgent/status`

### 5) Control y recarga

Dispare acciones operativas de control cuando haga falta.

- `POST /reload_libreqos`

Trátelas como acciones de mayor riesgo en producción.

## Modelo de persistencia (importante)

No todas las escrituras persisten igual:

- `overrides/*`: estado de política persistente.
- `shaped_devices/update`: reemplazo directo del archivo, a menudo sobrescrito por
  las integraciones.

Las escrituras v2 `network_json/set_site_speed*` no existen en v3 (`404` JSON);
los cambios de velocidad de sitio son estado de override persistente vía
`POST /overrides/network_adjustments/site_speed`.

Si las integraciones están habilitadas, sus ciclos de refresco pueden sobrescribir
ediciones directas de archivos.

## Flujo de trabajo recomendado en producción

1. Valide el comportamiento y el esquema del payload en Swagger.
2. Aplique el cambio más pequeño posible.
3. Verifique el resultado con comprobaciones de solo lectura (`/health`,
   `/scheduler_status`, vistas de circuito y throughput).
4. Guarde copias de respaldo antes de operaciones que cambian configuración o
   datos.

## Endurecimiento del despliegue

- Limite el acceso a la API a redes de gestión de confianza.
- No exponga el listener del gestor del nodo directamente a Internet.
- Si necesita acceso remoto, use un proxy inverso con TLS (vea
  [HTTPS opcional con Caddy](https-caddy-es.md)).
- Restrinja el acceso entrante con listas de permitidos en el firewall.
