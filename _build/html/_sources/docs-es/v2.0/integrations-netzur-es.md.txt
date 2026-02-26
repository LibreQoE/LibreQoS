# Integración con Netzur

## Resumen

Use esta integración cuando Netzur sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure `[netzur_integration]` en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- La integración regenera `ShapedDevices.csv`.
- `network.json` depende de su configuración de sobrescritura.

## Referencia completa

- [Referencia detallada de Netzur](integrations-reference-es.md#integración-con-netzur)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
