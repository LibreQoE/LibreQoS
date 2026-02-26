# Alta Disponibilidad y Dominios de Falla

Esta página describe un modelo práctico de activo/respaldo para LibreQoS.

## Alcance y Supuestos

- Esta página cubre alta disponibilidad de LibreQoS con un diseño activo/respaldo.
- La conmutación por falla debe controlarse con enrutamiento dinámico (por ejemplo OSPF o BGP).
- No se prescribe ningún fabricante de hardware o router.
- La alta disponibilidad específica de Insight está fuera del alcance de esta página.

## Modelo HA Activo/Respaldo

Una ruta está activa (preferida) y otra está en respaldo (espera). La política de enrutamiento controla la selección de ruta y la conmutación por falla.

## Ejemplo OSPF (Costo 1 Primario, Costo 100 Respaldo)

Use el costo de interfaz OSPF para preferir la ruta activa.

- Ruta primaria de LibreQoS: `ip ospf cost 1`
- Ruta de respaldo de LibreQoS: `ip ospf cost 100`

Resultado conceptual:

- Estado normal: el tráfico usa la ruta primaria porque el costo `1` es menor que `100`.
- Estado de falla: si la ruta/router primaria cae, OSPF converge y el tráfico usa la ruta de respaldo.
- Estado de recuperación: cuando la primaria vuelve a estar sana, el tráfico regresa a la primaria por su menor costo.

Intención de política:

1. Mantener ambas rutas activas y enrutables todo el tiempo.
2. Asegurar que el respaldo tenga capacidad suficiente para el pico esperado.
3. Probar conmutación por falla y retorno en ventanas de mantenimiento.

## Equivalente en BGP (Si Usa BGP en Lugar de OSPF)

Si usa BGP, utilice controles de preferencia estándar para definir una ruta primaria y otra de respaldo (por ejemplo local preference, MED o AS path prepending según su diseño). Mantenga la política determinista y documentada.

## Recuperación y Retorno (Runbook Conceptual)

1. Confirmar si ocurrió la conmutación por falla (tabla de rutas/verificación de camino).
2. Verificar que el tráfico de clientes esté fluyendo por la ruta de respaldo.
3. Reparar la ruta primaria fallida.
4. Validar la salud de la ruta primaria.
5. Restaurar la preferencia de rutas a su estado normal (primaria preferida).
6. Verificar que el tráfico regresó a la primaria y que el rendimiento es estable.

## Mantenimiento Planificado (Procedimiento Conceptual)

1. Anunciar ventana y criterios de éxito.
2. Confirmar salud y capacidad de la ruta de respaldo.
3. Mover tráfico al respaldo usando política de enrutamiento.
4. Realizar mantenimiento en la ruta activa anterior.
5. Validar la ruta reparada.
6. Opcionalmente regresar el tráfico al estado activo normal.
7. Cerrar la ventana con notas de validación posteriores al cambio.

## Lista de Verificación de Preparación HA

- El enrutamiento dinámico está implementado y documentado.
- Las preferencias activo/respaldo son explícitas y probadas.
- Monitoreo y alertas cubren ambas rutas y dependencias clave.
- El runbook de guardia incluye pasos de failover y failback.
- Existe una cadencia de pruebas regulares (por ejemplo pruebas trimestrales).
- La capacidad de la ruta de respaldo está validada para carga pico realista.

## Límites Conocidos

- La alta disponibilidad depende de la calidad del diseño de red circundante.
- La convergencia del enrutamiento dinámico no es instantánea.
- Una política mal configurada puede causar failover asimétrico o inestable.
- HA no reemplaza respaldos ni disciplina operativa.

## Documentación Relacionada

- [Planificación de Escala y Diseño de Topología](scale-topology-es.md)
- [Ajuste de rendimiento](performance-tuning-es.md)
- [StormGuard](stormguard-es.md)
- [Configuración](configuration-es.md)
- [Integraciones](integrations-es.md)
- [Resolución de Problemas](troubleshooting-es.md)
