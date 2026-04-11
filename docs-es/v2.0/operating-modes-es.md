# Modos de operación y fuente de verdad

## Propósito de esta página

Use esta página para decidir dónde deben hacerse los cambios permanentes de shaping antes de pasar a producción.

¿Necesita definiciones de términos usados aquí? Vea el [Glosario](glossary-es.md).

LibreQoS soporta tres modos de operación comunes.

## Integraciones incluidas (recomendado para la mayoría de operadores)

En este modo, su CRM/NMS es el lugar donde deben hacerse los cambios permanentes de topología y suscriptores, y LibreQoS se mantiene sincronizado desde allí.

Comportamiento clave:
- La sincronización de la integración refresca los datos de topología y shaping usados por LibreQoS.
- `network.json` queda reservado para despliegues DIY o manuales.
- Las ediciones manuales directas pueden sobrescribirse en el siguiente refresco del scheduler.
- El modo de topología `flat` simplifica el árbol cuando necesita menor sobrecarga.

## Fuente de verdad personalizada

En este modo, sus propios scripts o sistemas generan `network.json` y `ShapedDevices.csv`.

Comportamiento clave:
- Su flujo externo es donde deben hacerse los cambios permanentes.
- Las ediciones por WebUI son válidas para cambios operativos rápidos.
- Mantenga los cambios de largo plazo en sus scripts o automatización.

## Modo archivos manuales

En este modo, usted mantiene `network.json` y `ShapedDevices.csv` directamente.

Comportamiento clave:
- Es más adecuado para redes pequeñas, pilotos cortos o soluciones temporales.
- La WebUI le ayuda a validar lo que LibreQoS está usando.
- Requiere disciplina manual porque no hay un sistema superior que mantenga esos archivos sincronizados.

## Lista de verificación de modo (antes de producción)

1. Elija un lugar principal para los cambios permanentes.
2. Confirme qué sistema escribe los datos de shaping en producción.
3. Confirme el comportamiento de refresco del scheduler y la cadencia de sobrescritura.
4. Documente su flujo de cambios rápidos (WebUI, editor externo, o ambos).
5. No mantenga ediciones en competencia en varios sistemas para los mismos objetos.

## Expectativas de topología y modo

- Diseños single-interface (on-a-stick) y con VLAN son válidos, pero requieren validación explícita de colas/interfaces después de cambios.
- El modo integración es ideal cuando CRM/NMS debe controlar topología y datos de suscriptores.
- Si necesita una topología que su integración no puede representar, use modo fuente personalizada y mantenga clara la responsabilidad.

Vea también:
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)

Si está usando integraciones incluidas, continúe en [Integraciones CRM/NMS](integrations-es.md).

## Páginas relacionadas

- [Configurar LibreQoS](configuration-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
