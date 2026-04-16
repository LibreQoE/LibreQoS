# Interfaz WebUI (Node Manager) de LibreQoS

Esta página documenta las vistas clave de la WebUI (Node Manager) y su comportamiento operativo en la interfaz local. Por defecto los operadores usan `http://ip_del_shaper:9123`. Si se habilita HTTPS opcional con Caddy, los operadores usan `https://tu-hostname/` o `https://tu-ip-de-gestión/`.

## Vistas principales

### Dashboard
- Resumen por widgets de throughput, retransmisiones, RTT, flujos y actividad de colas.
- El contenido puede variar según versión y funciones habilitadas.
- Executive Summary ofrece una vista operativa compacta para redes grandes, con un `Network Snapshot` centrado en throughput, inventario y estado de Insight, además de páginas de detalle para heatmaps y rankings ejecutivos.
- Bakery ofrece una pestaña dedicada para el estado de aplicación de colas, resultados de seguridad/preflight de qdisc, progreso de cambios en vivo sobre circuitos y operaciones recientes de Bakery.
- Las pestañas de Bakery y TreeGuard presentan un resumen de alto nivel del pipeline o control loop antes de las tablas más detalladas.
- El widget `Pipeline` de Bakery muestra las etapas del control de colas, el estado de aplicación, el estado de verificación y la temporización del intervalo de TC.
- `Runtime Operations` resume mutaciones de topología entre TreeGuard y Bakery, trabajo de limpieza diferida, fallos y subárboles que esperan un full reload.
- `Recent Bakery Events` prioriza operaciones agrupadas, con el historial detallado disponible cuando hace falta troubleshooting más profundo.
- `TreeGuard Activity` prioriza operaciones agrupadas, incluyendo lotes de cambios SQM, con el historial detallado disponible cuando hace falta troubleshooting más profundo.
- `TreeGuard Control Loop` muestra el estado actual de observar/evaluar/actuar.
- `TreeGuard Decision Impact` se centra en el impacto actual y en las advertencias o errores activos.
- `TreeGuard State Mix` muestra nodos gestionados, virtualización en runtime, circuitos gestionados y la mezcla de circuitos `cake / mixed / fq_codel`.
- El preflight de qdisc de Bakery resume el uso planificado de qdisc por interfaz y el margen de presupuesto antes de aplicar cambios.
- Algunos gráficos pueden tardar un poco en poblarse al abrir una pestaña por primera vez, especialmente en sistemas ocupados o inmediatamente después de reiniciar servicios.
- Durante un full reload de Bakery, las tarjetas de conteo de colas pueden seguir mostrando temporalmente los últimos valores conocidos de HTB/CAKE/fq-codel y marcarlos como `Reloading`.

### Vista de árbol de red
- Vista jerárquica de nodos/circuitos desde la perspectiva del shaper.
- Útil para identificar cuellos de botella y patrones de utilización padre/hijo.
- Las páginas de detalle del árbol muestran una ruta tipo breadcrumb, conteos de rama e indicadores de estado para el nodo seleccionado.
- `Node Details` resume las velocidades configuradas del nodo seleccionado, el estado de overrides y la velocidad efectiva.
- `Node Snapshot` ofrece un resumen visual rápido del throughput y el QoO del nodo seleccionado.
- Los circuitos adjuntos se muestran en una tabla dedicada para el nodo seleccionado.
- La columna de IP de circuitos adjuntos mantiene las filas compactas mostrando una dirección inline y colapsando las adicionales como `+X`, mientras la lista completa sigue disponible al pasar el cursor.
- Los circuitos adjuntos limitados por Ethernet pueden mostrar insignias `10M`, `100M` o `1G` junto al valor de `Plan (Mbps)`; al pasar el cursor se explica el auto-cap y al hacer clic en la insignia se abre la página dedicada de revisión Ethernet.
- Los administradores pueden guardar o limpiar valores de `Rate Override` cuando el nodo admite overrides a nivel de nodo. Los usuarios de solo lectura y los nodos no compatibles siguen mostrando los valores actuales sin controles de edición.
- Las ediciones de tasa desde la página del árbol escriben overrides operativos `AdjustSiteSpeed` en `lqos_overrides.json`.
- En compilaciones UISP `full`, un `integrationUISPbandwidths.csv` heredado se auto-migra a esos overrides en la siguiente ejecución de la integración cuando todavía no existen overrides operativos de tasa; en caso contrario, el CSV se ignora.
- Las ediciones de tasa desde la página del árbol requieren sesión de administrador, un `node_id` estable y un nodo no generado. Los nodos generados/sintéticos de la integración permanecen en solo lectura en este editor.
- La página del árbol mantiene `Node Details` como una tarjeta compacta de resumen y coloca los editores de override como filas compactas directamente debajo de la tabla de detalles.
- La página del árbol ahora muestra el estado de `Topology Override` solo en modo lectura. Los cambios de padre y adjunto se movieron a `Topology`, y el panel de detalles del árbol enlaza directamente a Topology Manager para el nodo seleccionado.
- Los saltos de adjuntos seleccionados en runtime se colapsan fuera de la jerarquía principal del árbol. Cuando un sitio usa actualmente un backhaul o radio path específico, `Node Details` lo muestra como metadato `Active Attachment` en el sitio en lugar de exponer ese adjunto como un nodo propio del árbol.
- Los adjuntos alternativos inactivos de backhaul PtP/cableado también se eliminan del árbol runtime. `tree.html` muestra solo la ruta efectiva; los alternativos inactivos quedan visibles en Topology Manager y no como nodos runtime independientes.
- En el nodo sintético `Root`, el topology override se sigue mostrando como no aplicable en lugar de una advertencia genérica por falta de `node_id`.
- El panel derecho de Details en Topology ahora usa un resumen compacto de rama más una sola sección centrada en adjuntos, de modo que `Current Attachment Preference`, `Attachment Health` y la ruta de `Edit Attachment Preference` quedan juntos en lugar de repetir el mismo estado en varias tarjetas.
- Para nodos con múltiples radio paths, el panel Details ahora prioriza la decisión de adjunto antes que el movimiento de rama: las tarjetas resaltan `Using Now` y `Preferred` directamente, mientras `Start Move` queda visualmente degradado hasta que el operador realmente quiera reubicar la rama.
- Mientras la página está abierta, Topology Manager sigue auto-refrescando el estado en segundo plano para mostrar cambios de health o suppress sin recargar manualmente, pero ahora difiere ese refresh cuando el operador está escribiendo en campos editables del panel Details para no robar foco ni cursor a mitad de una edición.
- La selección actual también se refleja en la URL, de modo que recargar o compartir el enlace reabre Topology Manager en el mismo nodo cuando ese nodo sigue existiendo. Si la página se abre sin `node_id`, ahora arranca por defecto en la vista sintética `Root` antes de volver al selector jerarquizado.
- En Move Preview, la ascendencia profunda hacia la izquierda ahora se compacta en un stub después de los dos nodos upstream más cercanos, para que las cadenas largas no aplasten el lado izquierdo del mapa. El breadcrumb completo sigue visible en el resumen de jerarquía superior.
- Cuando UISP aporta adjuntos/radios explícitos, los destinos automáticos de sondeo de topología salen de las IPs de gestión que UISP reporta para ese par. Estas IPs de sondeo ya no están limitadas por `allow_subnets` de shaping; se tratan como datos del plano de gestión y no como direcciones de clientes para shaping.
- La sección `Attachment Health` también puede guardar overrides de tasa por adjunto cuando ese adjunto es editable. Estos overrides son direccionales (`download` / `upload`), viven en `topology_overrides.json` y solo afectan la ruta concreta `(nodo hijo, nodo padre, adjunto)`, no todo el nodo.
- En modo de adjunto `Auto`, Topology Manager usa los probes para suprimir enlaces conocidos como malos, pero no descalifica un enlace solo porque el probe esté deshabilitado o no disponible. Cuando quedan varios enlaces elegibles, `Auto` prioriza las tasas respaldadas por telemetría de integración antes que las tasas estáticas de respaldo y luego resuelve empates por capacidad.
- Las filas de adjuntos UISP también clasifican el rol de alimentación, como `PtP Backhaul`, `PtMP Uplink` o `Wired Uplink`, para que el operador distinga un backhaul real de una ruta de acceso/uplink sin deducirlo por el nombre del AP.
- El archivo único de depuración runtime para probes de topología es `topology_attachment_health_state.json`. Ahora incluye el `pair_id` del probe más el nombre/ID del adjunto, los nombres/IDs de nodo hijo y padre, las IPs local/remota configuradas para probe, el estado `enabled`/`probeable`, la alcanzabilidad reciente por endpoint y los contadores/razones actuales de health o suppress. El panel Details enlaza directamente a esta depuración desde `Attachment Health`.
- Los adjuntos UISP cuya capacidad viene de telemetría dinámica de radio siguen en solo lectura para este editor. Los adjuntos estáticos, los casos black-box/fallback y los grupos manuales sí pueden mostrar controles de `Attachment Rate`.
- Cuando UISP limita automáticamente un adjunto porque los puertos Ethernet activos o conocidos no pueden transportar la capacidad bruta reportada por el radio, `Attachment Health` muestra esa razón inline para que el operador vea por qué un radio de 2G o 2.7G exportó una tasa efectiva menor en topología.
- `tree.html` solo compacta adjuntos UISP efectivos cuando el rol del adjunto es realmente de tipo backhaul (`PtP Backhaul` o `Wired Uplink`). Los APs PtMP de acceso/uplink permanecen visibles en el árbol runtime.

### Topology Probes
- Página de depuración en solo lectura para troubleshooting de probes de topología a nivel global, enlazada desde Topology Manager en lugar de figurar como un destino principal de la barra lateral.
- Carga desde el snapshot runtime único `topology_attachment_health_state.json`.
- Por defecto muestra solo probes habilitados, con filtros para cambiar a todos o solo deshabilitados cuando se necesite troubleshooting.
- Usa el mismo estilo denso de panel/configuración que otras páginas operativas de inventario, con una pequeña franja de resumen de estado encima de la tabla.
- Cada fila muestra nodo hijo, nodo padre, adjunto, IPs de probe, estado runtime de health/suppress, alcanzabilidad por endpoint y un enlace directo de vuelta a Topology Manager.

### Site Map
- Mapa operativo plano de sitios y APs usando geodatos importados de nodos.
- Usa QoO por defecto con un selector alternativo para RTT, mientras el tamaño del marcador refleja el throughput combinado reciente.
- Los APs pueden heredar coordenadas del sitio padre solo para visualización cuando faltan coordenadas explícitas.
- Los marcadores de sitios cercanos se agrupan y se expanden al acercar el mapa o seleccionar un grupo.
- Los APs sin coordenadas explícitas se representan a través de su sitio padre y pueden desplegarse temporalmente alrededor del sitio seleccionado para inspección.
- Los sitios visibles y sin agrupar muestran etiquetas al acercar el mapa, y el sitio seleccionado mantiene su etiqueta visible mientras se inspecciona.
- Cuando el modo de redacción del navegador está habilitado, Site Map reemplaza los nombres de sitios mostrados por `[redacted]` sin modificar los datos reales de topología.
- El encuadre inicial del mapa prioriza las coordenadas de los sitios para la vista inicial, usando coordenadas de AP solo cuando todavía no hay sitios mapeados.
- Site Map usa una capa raster de OpenStreetMap alojada por Insight.
- En modo oscuro, el mapa base se atenúa y se tiñe hacia una paleta azul/cian para mantener visibles carreteras y geografía sin el brillo del mapa claro.
- Site Map depende del acceso saliente a `https://insight.libreqos.com` para el arranque inicial del mapa y la obtención de tiles.

### Flow Globe
- Visualización geográfica de flujos basada en la geolocalización de endpoints.
- Usa un globo temático con bordes de países para contexto geográfico.
- Los marcadores de endpoints usan latencia por defecto, con un selector para cambiar entre latencia y throughput.
- El tamaño del marcador indica el volumen reciente de tráfico.
- Pase el cursor para detalles rápidos o haga clic en un marcador/cluster para fijar sus detalles en el panel lateral.
- Requiere volumen de datos reciente suficiente.

### ASN Analysis
- Página operativa ASN en vivo que combina un ranking top-20 de ASN, gráfico de burbujas latencia-vs-tráfico, franja mínima de KPIs del ASN seleccionado, gráfico de tendencia ASN de 15 minutos y la sección integrada de Flow Evidence.
- Soporta modos de ranking `Impact` y `Throughput`, manteniendo la evidencia de flujos ASN en la misma página.
- Los marcadores antiguos de `ASN Explorer` siguen funcionando mediante redirección.
- Resultados vacíos suelen indicar poco dato reciente, no necesariamente falla.

### Página de circuito
- Las páginas de circuito combinan comportamiento de colas, throughput en vivo, RTT, retransmisiones y troubleshooting por flujo para un suscriptor/circuito individual.
- Cuando los metadatos de la integración informan la velocidad Ethernet negociada del CPE, la fila `Max` puede mostrar una insignia de advertencia como `100M`; al pasar el cursor sobre la insignia se explica cuándo LibreQoS redujo automáticamente el shaping por debajo del plan solicitado para respetar ese límite del puerto, y al hacer clic en la insignia se abre la página de revisión Ethernet.
- `Queue Dynamics` muestra el comportamiento del throughput y RTT del circuito a lo largo del tiempo, incluyendo un KPI de `Active Flows` basado en la misma ventana reciente usada por la tabla `Traffic Flows`.
- `Top ASNs` resume los ASN recientes más activos del circuito a partir de esa misma ventana de flujos en vivo y ordena por tasa actual por defecto.
- `Devices` muestra tablas de detalle por dispositivo y gráficos en vivo de throughput, retransmisiones y latencia.
- `Queue Stats` muestra historial reciente de la cola del circuito, incluyendo backlog, delay, longitud de cola, tráfico, marcas ECN y drops.
- Los gráficos de Queue Stats usan hover sincronizado para inspeccionar el mismo segundo en todos los gráficos de cola al mismo tiempo.
- `Queue Stats` debe seguir mostrando la telemetría en vivo de la cola del circuito en modo `flat` igual que en los demás modos de topología.
- `Queue Tree` muestra la ruta ascendente de colas del circuito, incluyendo un resumen de ruta y contexto de throughput, retransmisiones y latencia para cada nodo aguas arriba.
- `Traffic Flows` es una tabla operativa de flujos recientes, no una vista de historial a largo plazo.
- `Traffic Flows` incluye paginación y un filtro `Hide Small Flows` para que los circuitos grandes y ocupados sigan siendo utilizables sin intentar renderizar cada fila.
- La tasa actual de `Traffic Flows` se limita a valores plausibles y coherentes con el plan del circuito.
- El texto largo en las columnas `Protocol`, `ASN` y `Country` se recorta con puntos suspensivos para mantener estable la altura de cada fila; el valor completo sigue disponible al pasar el cursor.
- `Flow Sankey` enfatiza los flujos recientes más activos en lugar de todos los flujos retenidos más antiguos.

### Ethernet Caps
- La página de revisión Ethernet es una tabla ligera para operadores con los circuitos reducidos automáticamente porque la velocidad Ethernet detectada quedó por debajo del plan solicitado.
- Intencionalmente no aparece en la navegación principal; los operadores llegan a ella haciendo clic en las insignias de advertencia Ethernet de la página de circuito o de la tabla de circuitos adjuntos del árbol.
- La página soporta búsqueda, filtro por tier (`10M`, `100M`, `1G+`) y paginación sobre los circuitos auto-capped.

### Árbol/ponderación de CPU
- Muestra distribución de colas/circuitos por núcleo de CPU.
- Ayuda a validar comportamiento de binpacking y balance de carga.
- CPU Affinity comienza mostrando solo los CPUs de shaping, y los núcleos excluidos o solo-del-host pueden mostrarse cuando hace falta.

### Editor de Shaped Devices
- Editor CRUD para `ShapedDevices.csv`.
- Incluye paginación y filtros.
- En el editor dedicado, las acciones de agregar, editar y eliminar se guardan de inmediato.

### Páginas de configuración
- Las contraseñas, los tokens API y las claves guardadas permanecen ocultos al abrir una página de integración.
- Si ya existe una credencial guardada, el campo permanece vacío hasta que escriba un reemplazo. Use `Clear` solo cuando quiera quitar el valor guardado.
- Deje el campo vacío para conservar el valor actual. Escriba uno nuevo para reemplazarlo.
- `Configuration -> SSL Setup` puede instalar Caddy, poner la WebUI detrás de HTTPS y luego desactivar ese HTTPS si quiere volver al HTTP directo en el puerto `9123`.

### Problemas urgentes
- WebUI puede mostrar problemas operativos urgentes informados por los servicios de LibreQoS.
- Ejemplos: advertencias de límites de mapeo/licencia y errores de alta prioridad.
- Operadores pueden reconocer/limpiar eventos desde la UI.
- Códigos comunes: `MAPPED_CIRCUIT_LIMIT` y `TC_U16_OVERFLOW` (ver [Solución de Problemas](troubleshooting-es.md#códigos-de-problemas-urgentes-y-primeras-acciones)).

### Estado del scheduler
- WebUI muestra salud/disponibilidad del scheduler.
- Úselo para validar refrescos periódicos después de cambios de configuración/integración.
- Si hay errores, correlacione con:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - `journalctl -u lqosd --since "30 minutes ago"`

## Modo privacidad / redacción

- Se activa con el ícono de máscara en la barra superior.
- Es un ajuste visual que se guarda en su navegador.
- Ayuda a ocultar PII en capturas/demos.
- Site Map reemplaza los nombres de sitios mostrados por `[redacted]` mientras el modo de redacción está activo.
- No cambia sus datos guardados ni sus archivos de configuración.

## Comportamiento de vistas vacías

Las siguientes vistas pueden verse vacías cuando hay poco dato:
- Site Map
- Flow Globe
- Sankey del árbol de red
- ASN Analysis / Flow Evidence

Si ocurre:
1. Confirme que `lqosd` está saludable.
2. Espere a que se acumule tráfico/dato reciente.
3. Recargue la página.
4. Revise logs:

```bash
journalctl -u lqosd --since "10 minutes ago"
```

## Páginas relacionadas

- [Componentes](components-es.md)
- [Configuración](configuration-es.md)
- [HTTPS opcional con Caddy](https-caddy-es.md)
- [Solución de Problemas](troubleshooting-es.md)
