# Interfaz WebUI (Node Manager) de LibreQoS

Esta página documenta las vistas clave de la WebUI (Node Manager) y su comportamiento operativo en la interfaz local (`http://ip_del_shaper:9123`).

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
- `Node Details` resume el tipo de nodo seleccionado, el tamaño de la rama, las velocidades configuradas y la velocidad efectiva.
- `Node Snapshot` ofrece un resumen visual rápido del throughput y el QoO del nodo seleccionado.
- Los circuitos adjuntos se muestran en una tabla dedicada para el nodo seleccionado.
- La columna de IP de circuitos adjuntos mantiene las filas compactas mostrando una dirección inline y colapsando las adicionales como `+X`, mientras la lista completa sigue disponible al pasar el cursor.
- Los circuitos adjuntos limitados por Ethernet pueden mostrar insignias `10M`, `100M` o `1G` junto al valor de `Plan (Mbps)`; al pasar el cursor se explica el auto-cap y al hacer clic en la insignia se abre la página dedicada de revisión Ethernet.
- Los administradores pueden guardar o limpiar valores de `Rate Override` cuando el nodo admite overrides a nivel de nodo. Los usuarios de solo lectura y los nodos no compatibles siguen mostrando los valores actuales sin controles de edición.
- Las ediciones de tasa desde la página del árbol escriben overrides operativos `AdjustSiteSpeed` en `lqos_overrides.json`.
- En compilaciones UISP `full`, un `integrationUISPbandwidths.csv` heredado se auto-migra a esos overrides en la siguiente ejecución de la integración cuando todavía no existen overrides operativos de tasa; en caso contrario, el CSV se ignora.
- Las ediciones de tasa desde la página del árbol requieren sesión de administrador, un `node_id` estable y un nodo no generado. Los nodos generados/sintéticos de la integración permanecen en solo lectura en este editor.
- La página del árbol mantiene `Node Details` como una tarjeta compacta de resumen y coloca los editores de override como filas compactas directamente debajo de la tabla de detalles.
- Los nodos de UISP con estrategia `full` también pueden exponer un editor de `Topology Override` en la página del árbol. Las compilaciones actuales soportan solo `Pinned Parent`, usando los candidatos inmediatos detectados aguas arriba del nodo y guardando esa decisión del operador en `lqos_overrides.json`.
- El selector de padre fijado usa `Default upstream parent` cuando no existe un override. Elegir esa opción deja el nodo con el padre seleccionado por la integración.

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

### Problemas urgentes
- WebUI puede mostrar problemas operativos urgentes emitidos por servicios backend.
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
- Es redacción del lado cliente y se guarda en `localStorage` del navegador.
- Ayuda a ocultar PII en capturas/demos.
- Site Map reemplaza los nombres de sitios mostrados por `[redacted]` mientras el modo de redacción está activo.
- No modifica `ShapedDevices.csv`, `network.json` ni datos backend.

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
- [Solución de Problemas](troubleshooting-es.md)
