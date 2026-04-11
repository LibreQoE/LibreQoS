# Integración con Sonar

## Resumen

Use esta integración cuando Sonar sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Sonar en `/etc/lqos.conf`.
2. Ejecute una importación manual de prueba.
3. Habilite sincronización por scheduler.

## Notas operativas

- La integración refresca automáticamente los datos importados y de shaping que LibreQoS usa con Sonar.
- `network.json` queda para despliegues DIY o manuales.
- Use la WebUI para confirmar que la importación terminó correctamente.

## Referencia completa

- [Referencia detallada de Sonar](integrations-reference-es.md#integración-con-sonar)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
