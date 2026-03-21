# Interfaz WebUI (Node Manager) de LibreQoS

Esta página documenta las vistas clave de la WebUI (Node Manager) y su comportamiento operativo en la interfaz local (`http://ip_del_shaper:9123`).

## Vistas principales

### Dashboard
- Resumen por widgets de throughput, retransmisiones, RTT, flujos y actividad de colas.
- El contenido puede variar según versión y funciones habilitadas.

### Vista de árbol de red
- Vista jerárquica de nodos/circuitos desde la perspectiva del shaper.
- Útil para identificar cuellos de botella y patrones de utilización padre/hijo.
- Las páginas de detalle del árbol ahora muestran una ruta tipo breadcrumb hasta el nodo seleccionado, para navegar hacia abajo sin depender de volver siempre a la raíz.
- Las filas del árbol ahora incluyen resúmenes del subárbol para `Sites` descendientes y `Circuits` adjuntos/descendientes, lo que ayuda a estimar el tamaño de cada rama antes de expandirla.
- Las filas del árbol y el encabezado del nodo seleccionado pueden mostrar pequeños íconos de estado para manejo especial, incluyendo nodos virtuales y nodos administrados actualmente por StormGuard.
- Las páginas de detalle del árbol incluyen una tarjeta de `Node Details` que muestra:
  - el tipo de nodo y el tamaño de la rama del nodo seleccionado
  - la velocidad configurada base desde el `network.json` generado
  - cualquier override de velocidad del operador almacenado en `lqos_overrides.json`
  - la velocidad configurada efectiva actual
- Las páginas de detalle del árbol también incluyen una barra de contexto compacta, un panel de medidores `Node Snapshot` y un indicador de advertencia de compatibilidad junto a `Operator Override` cuando existen avisos de compatibilidad relacionados con overrides de integraciones.
- Las páginas de detalle del árbol ahora incluyen un localizador estable del nodo en la URL cuando está disponible y vuelven a resolver el nodo seleccionado ante reindexaciones en vivo del árbol, para que permanecer en una página siga el mismo nodo lógico después de regenerar `network.json`.
- Los administradores pueden guardar o limpiar overrides de velocidad del nodo desde el editor `Operator Override` cuando el nodo seleccionado es editable. Los usuarios de solo lectura y los nodos no editables siguen viendo los valores actuales en la tarjeta de detalles.
- El nodo sintético `Root` se muestra como un agregado de solo lectura y no expone el editor `Operator Override`.
- Los overrides de velocidad desde el árbol requieren un ID estable del nodo y rechazan intencionalmente nodos generados. Cuando un override ya fue guardado pero todavía no fue materializado en el `network.json` generado, la tarjeta de detalles muestra un indicador `⟳ Pending` explicando que el cambio se aplicará en la siguiente ejecución del scheduler.
- El control de pausa del Sankey de árbol completo ahora solo detiene el polling; el drill-down local, el reset y los cambios de profundidad máxima siguen renderizando desde el snapshot en caché mientras está en pausa.

### Site Map
- Mapa operativo plano de sitios y APs usando geodatos importados de nodos.
- Usa QoO por defecto con un selector alternativo para RTT, mientras el tamaño del marcador refleja el throughput combinado reciente.
- Usa un promedio del lado cliente de 30 segundos a partir de `NetworkTree`, sin agregar trabajo de rollup en el backend.
- Los APs pueden heredar coordenadas del sitio padre solo para visualización cuando faltan coordenadas explícitas.
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
- La ruta heredada `ASN Explorer` ahora redirige aquí para conservar compatibilidad con marcadores antiguos.
- Resultados vacíos suelen indicar poco dato reciente, no necesariamente falla.

### Árbol/ponderación de CPU
- Muestra distribución de colas/circuitos por núcleo de CPU.
- Ayuda a validar comportamiento de binpacking y balance de carga.

### Editor de Shaped Devices
- Editor CRUD para `ShapedDevices.csv`.
- Incluye paginación y filtros en versiones actuales.

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
