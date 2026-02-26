# Interfaz WebUI (Node Manager) de LibreQoS

Esta página documenta las vistas clave de la WebUI (Node Manager) y su comportamiento operativo en la interfaz local (`http://ip_del_shaper:9123`).

## Vistas principales

### Dashboard
- Resumen por widgets de throughput, retransmisiones, RTT, flujos y actividad de colas.
- El contenido puede variar según versión y funciones habilitadas.

### Vista de árbol de red
- Vista jerárquica de nodos/circuitos desde la perspectiva del shaper.
- Útil para identificar cuellos de botella y patrones de utilización padre/hijo.

### Mapa de flujos
- Visualización geográfica de flujos según geolocalización de endpoints.
- Requiere volumen de datos reciente suficiente.

### Explorador ASN
- Exploración por ASN (volumen, RTT/retransmisiones y detalle de flujos asociados).
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
- Mapa de flujos
- Sankey del árbol de red
- Explorador ASN

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
