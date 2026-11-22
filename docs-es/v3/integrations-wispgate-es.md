# Integración con WISPGate

**Versión: LibreQoS v3.**

## Resumen

Use esta integración cuando WISPGate sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure WISPGate en `/etc/libreqos/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` sigue siendo una entrada DIY/manual operada por el usuario.
- Para despliegues guiados por integración, use `topology_import.json` y `topology_operator_view.json`.

## Referencia completa

- [Guía completa de WISPGate (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/integrations-wispgate.html)
- [Operación de LibreQoS v3 (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/operate.html)
