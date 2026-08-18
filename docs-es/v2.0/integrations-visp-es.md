# Integración con VISP

## Resumen

Use esta integración cuando VISP sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure credenciales `[visp_integration]` en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se reescribe en cada ejecución.
- La importación VISP usa GraphQL y prioriza las consultas bulk rápidas; si faltan suscriptores o servicios, intenta completar la cobertura con consultas adicionales de GraphQL.
- Cuando VISP IRM tiene pobladas las relaciones de dispositivo upstream, la importación también construye topología de sitio/upstream para esos suscriptores en lugar de quedarse siempre en modo plano.
- `network.json` sigue siendo una entrada DIY/manual operada por el usuario.
- Para despliegues guiados por integración, use `topology_import.json` y `network.effective.json`.

## Referencia completa

- [Referencia detallada de VISP](integrations-reference-es.md#integración-con-visp)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
