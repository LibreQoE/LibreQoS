# Integraciones CRM/NMS

La mayoría de operadores usan las integraciones incluidas. Si usa scripts propios como fuente de verdad para `network.json` y `ShapedDevices.csv`, comience por [Modos de operación y fuente de verdad](operating-modes-es.md).

## Elegir ruta de integración

| Ruta | Mejor para | Fuente de verdad principal |
|---|---|---|
| Integración incluida | La mayoría de operadores con sistemas soportados | CRM/NMS por jobs de integración de LibreQoS |
| Fuente-de-verdad personalizada | Operadores con sincronización propia de CRM/NMS | Archivos externos generados (`network.json`, `ShapedDevices.csv`) |

## Dónde en la WebUI

- Comportamiento común de integración: `Configuration -> Integrations`
- Campos por integración: `Configuration -> Integrations`
- Validación de salud tras cambios: `WebUI -> Scheduler Status` y `WebUI -> Urgent Issues`
- Validación de resultados/topología: `WebUI -> Network Tree Overview` y `Flow Map`

## Integraciones incluidas

- [Integración con Splynx](integrations-splynx-es.md)
- [Integración con UISP](integrations-uisp-es.md)
- [Integración con Netzur](integrations-netzur-es.md)
- [Integración con VISP](integrations-visp-es.md)
- [Integración con WISPGate](integrations-wispgate-es.md)
- [Integración con Powercode](integrations-powercode-es.md)
- [Integración con Sonar](integrations-sonar-es.md)

## Comportamiento importante de sobrescritura

Cuando hay integraciones habilitadas:
- `ShapedDevices.csv` normalmente se regenera por jobs de sincronización.
- `network.json` también puede sobrescribirse según configuración (por ejemplo `always_overwrite_network_json`).
- Las ediciones manuales pueden sobrescribirse en el siguiente ciclo de refresco.

## ¿Necesita la referencia completa heredada?

El contenido detallado completo se conserva aquí:
- [Referencia de integraciones (detallada)](integrations-reference-es.md)

## Páginas relacionadas

- [Quickstart](quickstart-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Configurar LibreQoS](configuration-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)
