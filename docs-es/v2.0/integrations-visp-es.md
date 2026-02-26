# Integración con VISP

## Resumen

Use esta integración cuando VISP sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure credenciales `[visp_integration]` en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se reescribe en cada ejecución.
- `network.json` se sobrescribe solo cuando está habilitado.
- Recomendado: mantener `always_overwrite_network_json = true` para despliegues guiados por integración VISP.

## Referencia completa

- [Referencia detallada de VISP](integrations-reference-es.md#integración-con-visp)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
