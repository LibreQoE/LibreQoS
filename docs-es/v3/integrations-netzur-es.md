# Integración con Netzur

**Versión: LibreQoS v3.**

## Resumen

Use esta integración cuando Netzur sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure `[netzur_integration]` en `/etc/libreqos/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- La integración refresca los datos importados y de shaping que LibreQoS usa con Netzur.
- `network.json` queda para despliegues DIY o manuales.
- Use la WebUI para validar que la importación terminó correctamente.

## Referencia completa

- [Guía completa de Netzur (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/integrations-netzur.html)
- [Operación de LibreQoS v3 (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/operate.html)
