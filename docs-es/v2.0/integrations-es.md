# Integraciones CRM/NMS

## Propósito de esta página

Use esta página para elegir y configurar integraciones CRM/NMS soportadas. Use las páginas por integración para detalles específicos.

¿Necesita definiciones de términos comunes? Vea el [Glosario](glossary-es.md).

La mayoría de operadores usan las integraciones incluidas.
Si todavía no eligió ruta de despliegue, empiece por [Quickstart](quickstart-es.md).

## Elegir ruta de integración

| Ruta | Mejor para | Dónde deben hacerse los cambios permanentes |
|---|---|---|
| Integración incluida | La mayoría de operadores con sistemas soportados | CRM/NMS por jobs de integración de LibreQoS |
| Fuente de verdad personalizada | Operadores con sincronización propia de CRM/NMS | Archivos externos generados (`network.json`, `ShapedDevices.csv`) |

## Dónde en la WebUI

- Comportamiento común de integración: `Configuration -> Integrations`
- Campos por integración: `Configuration -> Integrations`
- Validación de salud tras cambios: `WebUI -> Scheduler Status` y `WebUI -> Urgent Issues`
- Validación de resultados/topología: `WebUI -> Network Tree Overview` y `Flow Globe`

## Integraciones incluidas

- [Integración con Splynx](integrations-splynx-es.md)
- [Integración con UISP](integrations-uisp-es.md)
- [Integración con Netzur](integrations-netzur-es.md)
- [Integración con VISP](integrations-visp-es.md)
- [Integración con WISPGate](integrations-wispgate-es.md)
- [Integración con Powercode](integrations-powercode-es.md)
- [Integración con Sonar](integrations-sonar-es.md)

## Comportamiento importante de refresco

Cuando hay integraciones habilitadas:
- La sincronización de la integración refresca los datos de topología y shaping que LibreQoS usa.
- Las integraciones incluidas no usan `network.json` ni `ShapedDevices.csv` como sus archivos normales de salida.
- Si trabaja en modo DIY o manual, `network.json` y `ShapedDevices.csv` siguen siendo los archivos que usted mantiene.
- Las ediciones directas sobre datos gestionados por la integración pueden sobrescribirse en el siguiente ciclo.

## Soporte para IDs de nodos de topología

LibreQoS soporta un campo genérico opcional `"id"` en los nodos de `network.json`. Este campo está pensado para transportar identificadores estables del sistema de integración cuando sea posible. En la versión actual, el campo es informativo y todavía no es la clave autoritativa para shaping u overrides.

| Integración | Soporte de ID de nodo en `network.json` | Notas |
|---|---|---|
| UISP | Sí | Sitios/dispositivos reales de UISP exportan `id` genérico más la metadata existente `uisp_site` / `uisp_device`. Los nodos sintéticos de LibreQoS usan IDs generados estables. |
| Splynx | Sí | Los nodos de topología de network sites y AP/site exportan `id` genérico. |
| Sonar | Sí | Los nodos de topología de sitios y AP exportan `id` genérico. |
| Netzur | Parcial | Se exporta solo cuando los datos upstream de zonas incluyen un ID de zona estable. |
| VISP | No | El importador actual shapea servicios/dispositivos pero no construye nodos de topología en `network.json`. |
| Powercode | No | El importador actual no construye nodos de topología en `network.json`. |
| WISPGate | No | El importador actual no construye nodos de topología a partir de identificadores estables upstream. |

## Manejo común de velocidades de cliente

Para las integraciones incluidas que importan velocidades brutas de plan de suscriptor, LibreQoS aplica la misma regla compartida antes de escribir `ShapedDevices.csv`:

- velocidad máxima efectiva del cliente = `max(plan_rate * bandwidth_overhead_factor, plan_rate * client_bandwidth_multiplier)`

Las integraciones que ya consumen velocidades efectivas de shaping conservan esos valores tal como llegan, sin volver a aplicar el multiplicador.

## Páginas relacionadas

- [Quickstart](quickstart-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Configurar LibreQoS](configuration-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)
