# Integración con Splynx

## Resumen

Use esta integración cuando Splynx sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Splynx en `/etc/lqos.conf`.
2. Seleccione estrategia de topología (`flat`, `ap_only`, `ap_site`, `full`).
3. Habilite sincronización automática y reinicie scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` depende de `always_overwrite_network_json`.
- Use WebUI para ajustes operativos diarios.

## Referencia completa

- [Referencia detallada de Splynx](integrations-reference-es.md#integración-con-splynx)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
