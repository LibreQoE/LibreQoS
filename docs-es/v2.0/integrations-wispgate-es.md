# Integración con WISPGate

## Resumen

Use esta integración cuando WISPGate sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure WISPGate en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` sigue siendo una entrada DIY/manual operada por el usuario.
- Para despliegues guiados por integración, use `topology_import.json` y `network.effective.json`.

## Referencia completa

- [Referencia detallada de WISPGate](integrations-reference-es.md#integración-con-wispgate)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
