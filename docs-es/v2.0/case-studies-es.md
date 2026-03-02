# Casos de Estudio (Anonimizados)

Esta pagina recopila historias cualitativas y anonimizadas de operadores para apoyar adopcion.

Politica de anonimización usada aqui:

- Geografia solo a nivel region/continente.
- Cantidades de suscriptores en bandas.
- Se pueden mencionar nombres de integraciones/productos.
- No se incluyen detalles unicos que identifiquen una red especifica.

## Historia 1: WISP Regional Estandariza en Integracion UISP

- Region: Norteamerica
- Banda de escala: 1,000-5,000 suscriptores
- Patron de despliegue: [Receta WISP/FISP](recipes-wisp-fisp-integration-es.md)

Situacion:

- Cambios frecuentes de planes generaban desviacion entre intencion y shaping activo.

Enfoque:

- UISP como fuente de verdad durable.
- `ShapedDevices.csv` controlado por integracion con politica de overwrite explicita.
- Estrategia topologica moderada antes de profundizar.

Resultado:

- Menos correcciones manuales tras cambios de planes.
- Onboarding operativo mas rapido.
- Comportamiento de colas mas predecible tras syncs recurrentes.

## Historia 2: Operador Maritimo Mejora Calidad con WAN Variable

- Region: rutas globales en multiples zonas oceanicas
- Banda de escala: 500-1,000 endpoints activos
- Patron de despliegue: [Receta Maritima StormGuard](recipes-maritime-stormguard-es.md)

Situacion:

- Variabilidad de capacidad WAN causaba oscilaciones de calidad en picos.

Enfoque:

- Modelo con un nodo top-level `Ship`.
- StormGuard en dry-run y luego activacion de ajustes acotados.
- Monitoreo de vistas debug/status en ventanas de carga.

Resultado:

- Mayor resiliencia de calidad durante congestion.
- Mejor visibilidad operacional de decisiones de limites.
- Proceso de cambios mas seguro por rollout escalonado.

## Historia 3: Red de Hospitalidad Migra a Equidad por Dispositivo

- Region: Europa
- Banda de escala: 500-1,000 habitaciones / 1,000-5,000 endpoints
- Patron de despliegue: [Receta Hospitalidad](recipes-hospitality-es.md)

Situacion:

- Shaping por habitacion no lograba equidad en alta ocupacion.

Enfoque:

- Mapeo por dispositivo en pools administrados.
- Jerarquia shallow y nombres parent estables.
- Seguimiento de memoria y presion queue/class antes de ampliar.

Resultado:

- Mejor percepcion de equidad entre dispositivos concurrentes.
- Mejor granularidad para troubleshooting de soporte.
- Mejor señal para planificacion de capacidad en picos.

## Paginas Relacionadas

- [Recetas de Despliegue](recipes-es.md)
- [Requisitos del Sistema](requirements-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
