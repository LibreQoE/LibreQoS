# Modos de operación y fuente de verdad

## Propósito de esta página

Use esta página para elegir y aplicar su política de fuente de verdad (integraciones incluidas vs fuente personalizada) antes de cambios en producción.

LibreQoS soporta dos modos de operación principales.

## Integraciones incluidas (recomendado para la mayoría de operadores)

En este modo, su CRM/NMS es la fuente de verdad y los jobs de integración de LibreQoS generan los insumos de shaping.

Comportamiento clave:
- `ShapedDevices.csv` se regenera en ciclos de sincronización.
- El comportamiento de sobrescritura de `network.json` depende de la configuración de integración (por ejemplo `always_overwrite_network_json`).
- Las ediciones manuales directas pueden sobrescribirse en el siguiente refresco del scheduler.

## Fuente-de-verdad personalizada

En este modo, sus propios scripts/sistemas generan `network.json` y `ShapedDevices.csv`.

Comportamiento clave:
- Su flujo externo controla la persistencia.
- Las ediciones por WebUI son válidas para cambios operativos rápidos.
- Los cambios permanentes deben mantenerse en su flujo externo de fuente de verdad.

## Lista de verificación de modo (antes de producción)

1. Elija una fuente de verdad principal.
2. Confirme qué sistema puede escribir insumos de shaping en producción.
3. Confirme el comportamiento de refresco del scheduler y la cadencia de sobrescritura.
4. Documente su flujo de cambios rápidos (WebUI, editor externo, o ambos).
5. No mantenga ediciones en competencia en varios sistemas para los mismos objetos.

## Expectativas de topología y modo

- Diseños single-interface (on-a-stick) y con VLAN son válidos, pero requieren validación explícita de colas/interfaces después de cambios.
- Modo integración es ideal cuando CRM/NMS debe controlar topología y datos de suscriptores.
- Si necesita topología estrictamente personalizada no representada por integración, use modo fuente personalizada y mantenga propiedad clara.

Vea también:
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Páginas relacionadas

- [Configurar LibreQoS](configuration-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
