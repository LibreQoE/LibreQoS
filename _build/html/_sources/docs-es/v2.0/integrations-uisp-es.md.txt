# Integración con UISP

## Resumen

Use esta integración cuando UISP sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure UISP en `/etc/lqos.conf`.
2. Elija estrategia de topología y estrategia de suspensión.
3. Habilite sincronización automática y reinicie scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` depende de `always_overwrite_network_json`.
- En modo integración, trate ediciones de archivos como temporales.

## Referencia completa

- [Referencia detallada de UISP](integrations-reference-es.md#integración-con-uisp)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
