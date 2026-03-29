# Interfaz WebUI (Node Manager) de LibreQoS

Esta página documenta las vistas clave de la WebUI (Node Manager) y su comportamiento operativo en la interfaz local (`http://ip_del_shaper:9123`).

## Vistas principales

### Dashboard
- Resumen por widgets de throughput, retransmisiones, RTT, flujos y actividad de colas.
- El contenido puede variar según versión y funciones habilitadas.
- Executive Summary ofrece una vista operativa compacta para redes grandes, con páginas de detalle para heatmaps y rankings ejecutivos.
- Bakery ofrece una pestaña dedicada para el estado de aplicación de colas, eventos recientes de Bakery paginados, resultados de seguridad/preflight de qdisc, progreso de cambios en vivo sobre circuitos y operaciones recientes de Bakery centradas en circuitos.
- Las pestañas de Bakery y TreeGuard incluyen un resumen de alto nivel del pipeline/control loop antes de las tablas detalladas.
- El widget `Pipeline` de Bakery muestra las etapas actuales del control de colas, incluyendo el estado activo de aplicación, el estado de verificación y la temporización del intervalo de TC.
- `Runtime Operations` resume mutaciones en vivo de topología entre TreeGuard y Bakery, trabajo de limpieza diferida, fallos, subárboles marcados como `Dirty` y si los cambios incrementales están congelados hasta un full reload.
- `Recent Bakery Events` muestra por defecto operaciones agrupadas orientadas al operador, y deja el log crudo de eventos de Bakery en una vista separada de `Event Log` cuando hace falta troubleshooting detallado.
- `TreeGuard Activity` muestra por defecto operaciones agrupadas orientadas al operador, incluyendo lotes consolidados de cambios SQM, y deja el log crudo de eventos de TreeGuard en una vista separada de `Event Log` cuando hace falta troubleshooting detallado.
- `TreeGuard Control Loop` muestra el estado actual de observar/evaluar/actuar sin repetir acciones recientes que ya aparecen en TreeGuard Activity.
- `TreeGuard Decision Impact` se centra en el impacto actual y en las advertencias o errores actuales, en lugar de repetir acciones recientes.
- `TreeGuard State Mix` muestra nodos gestionados, virtualización en runtime, circuitos gestionados y la mezcla actual de circuitos `cake / mixed / fq_codel`.
- El preflight de qdisc de Bakery resume el uso planificado de qdisc por interfaz y el margen de presupuesto antes de aplicar cambios.
- Algunos gráficos pueden tardar un poco en poblarse al abrir una pestaña por primera vez, especialmente en sistemas ocupados o inmediatamente después de reiniciar servicios.
- Durante un full reload de Bakery, las tarjetas de conteo de colas mantienen los últimos valores conocidos de HTB/CAKE/fq-codel y los marcan como `Reloading`.

### Vista de árbol de red
- Vista jerárquica de nodos/circuitos desde la perspectiva del shaper.
- Útil para identificar cuellos de botella y patrones de utilización padre/hijo.
- Las páginas de detalle del árbol muestran una ruta tipo breadcrumb, conteos de rama e indicadores de estado para el nodo seleccionado.
- `Node Details` resume el tipo de nodo seleccionado, el tamaño de la rama, las velocidades configuradas y la velocidad efectiva actual.
- `Node Snapshot` ofrece un resumen visual rápido del throughput actual y el QoO del nodo seleccionado.
- Los circuitos adjuntos se muestran en una tabla dedicada para el nodo seleccionado.
- Los administradores pueden guardar o limpiar valores de `Operator Override` cuando el nodo admite overrides a nivel de nodo. Los usuarios de solo lectura y los nodos no compatibles siguen mostrando los valores actuales sin controles de edición.

### Site Map
- Mapa operativo plano de sitios y APs usando geodatos importados de nodos.
- Usa QoO por defecto con un selector alternativo para RTT, mientras el tamaño del marcador refleja el throughput combinado reciente.
- Usa un promedio del lado cliente de 30 segundos a partir de `NetworkTree`, sin agregar trabajo de rollup en el backend.
- Los APs pueden heredar coordenadas del sitio padre solo para visualización cuando faltan coordenadas explícitas.
- Los marcadores de sitios cercanos se agrupan y se expanden al acercar el mapa o seleccionar un grupo.
- Los APs sin coordenadas explícitas se representan a través de su sitio padre y pueden desplegarse temporalmente alrededor del sitio seleccionado para inspección.
- Los sitios visibles y sin agrupar muestran etiquetas al acercar el mapa, y el sitio seleccionado mantiene su etiqueta visible mientras se inspecciona.
- Cuando el modo de redacción del navegador está habilitado, Site Map reemplaza los nombres de sitios mostrados por `[redacted]` sin modificar los datos reales de topología.
- El encuadre inicial del mapa prioriza las coordenadas de los sitios para la vista inicial, usando coordenadas de AP solo cuando todavía no hay sitios mapeados.
- En modo oscuro, la capa raster del mapa base se atenúa y se tiñe del lado cliente hacia una paleta azul/cian similar a la de Flow Globe, para mantener visibles carreteras y geografía sin el brillo del mapa claro.
- Usa un mapa base local con estilo LibreQoS con bordes de país/estado, costas, lagos principales, ríos principales, áreas marinas, superposiciones sutiles de regiones físicas y contexto de autopistas principales a mayor zoom para orientación geográfica.
- Site Map utiliza una capa local de carreteras derivada de Natural Earth para ayudar con la orientación, manteniendo el resto del mapa base discreto y operativo.

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
- El contexto ejecutivo ASN se obtiene mediante requests acotados al ASN, en lugar de suscribirse a un feed completo de heatmaps ejecutivos.
- Los marcadores antiguos de `ASN Explorer` siguen funcionando mediante redirección.
- Resultados vacíos suelen indicar poco dato reciente, no necesariamente falla.

### Página de circuito
- Las páginas de circuito combinan comportamiento de colas, throughput en vivo, RTT, retransmisiones y troubleshooting por flujo para un suscriptor/circuito individual.
- `Queue Dynamics` muestra el comportamiento del throughput y RTT del circuito a lo largo del tiempo, incluyendo un KPI de `Active Flows` basado en la misma ventana reciente usada por la tabla `Traffic Flows`.
- `Queue Stats` muestra los 3 minutos más recientes del historial en vivo de la cola del circuito como muestras scatter crudas de 1 segundo, incluyendo backlog, delay, longitud de cola, tráfico, marcas ECN y drops.
- Los gráficos de Queue Stats usan hover sincronizado para inspeccionar el mismo segundo en todos los gráficos de cola al mismo tiempo.
- `Queue Tree` muestra la ruta ascendente de colas del circuito, incluyendo un resumen de ruta y contexto de throughput, retransmisiones y latencia para cada nodo aguas arriba.
- `Traffic Flows` es una tabla operativa de flujos recientes, no una vista de historial a largo plazo.
- `Traffic Flows` incluye paginación y un filtro `Hide Small Flows` para que los circuitos grandes y ocupados sigan siendo utilizables sin intentar renderizar cada fila.
- `Flow Sankey` enfatiza los flujos recientes más activos en lugar de todos los flujos retenidos más antiguos.

### Árbol/ponderación de CPU
- Muestra distribución de colas/circuitos por núcleo de CPU.
- Ayuda a validar comportamiento de binpacking y balance de carga.
- CPU Affinity muestra por defecto el conjunto de CPUs de shaping detectado, de modo que los E-cores híbridos / núcleos del host excluidos queden ocultos salvo que el operador decida mostrarlos.

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
