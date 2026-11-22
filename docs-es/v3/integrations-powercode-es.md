# Integración con Powercode

**Versión: LibreQoS v3.**

## Resumen

Use esta integración cuando Powercode sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Powercode en `/etc/libreqos/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- Revise cómo desea gestionar `network.json` en su flujo de topología.

## Referencia completa

- [Guía completa de Powercode (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/integrations-powercode.html)
- [Operación de LibreQoS v3 (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/operate.html)
