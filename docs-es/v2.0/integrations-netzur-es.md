# Integración con Netzur

## Resumen

Use esta integración cuando Netzur sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure `[netzur_integration]` en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- La integración refresca los datos importados y de shaping que LibreQoS usa con Netzur.
- `network.json` queda para despliegues DIY o manuales.
- Use la WebUI para validar que la importación terminó correctamente.

## Referencia completa

- [Referencia detallada de Netzur](integrations-reference-es.md#integración-con-netzur)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
