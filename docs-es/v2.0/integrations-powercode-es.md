# Integración con Powercode

## Resumen

Use esta integración cuando Powercode sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Powercode en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- Revise cómo desea gestionar `network.json` en su flujo de topología.

## Referencia completa

- [Referencia detallada de Powercode](integrations-reference-es.md#integración-con-powercode)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
