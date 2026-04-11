# Integración con Splynx

## Resumen

Use esta integración cuando Splynx sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Splynx en `/etc/lqos.conf`.
2. Seleccione estrategia de topología (`flat`, `ap_only`, `ap_site`, `full`).
3. Habilite sincronización automática y reinicie scheduler.

## Inicio rápido para nuevos operadores

Base recomendada:

- `strategy = "ap_only"` (menos confusión inicial)
- `enable_splynx = true`
- `network.json` queda reservado para despliegues DIY o manuales

Después, ejecute una sincronización manual y valide salidas antes de aumentar frecuencia de refresh.

## Notas operativas

- La sincronización actualiza automáticamente los datos importados y de shaping que LibreQoS usa con Splynx.
- `network.json` queda para despliegues DIY o manuales.
- Use la WebUI para confirmar que la importación y la profundidad del árbol son las esperadas.
- Use WebUI para ajustes operativos diarios.

## Validación en 5 minutos después de cambios Splynx

1. Ejecute prueba de integración:
```bash
python3 integrationSplynx.py
```
2. Confirme archivos actualizados:
```bash
ls -lh /opt/libreqos/src/topology_import.json /opt/libreqos/src/shaping_inputs.json
```
3. Confirme salud de servicios:
```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
```
4. Verifique en WebUI que Scheduler Status y profundidad de árbol coincidan con la estrategia elegida.

## Referencia completa

- [Referencia detallada de Splynx](integrations-reference-es.md#integración-con-splynx)
- [Modos de operación y fuente de verdad](operating-modes-es.md)

Las versiones actuales también exponen una política compartida de margen para puertos Ethernet en `Configuration -> Integrations -> Integration Defaults`. Las integraciones que pueden aportar la velocidad Ethernet negociada hacia el suscriptor usan un multiplicador conservador por defecto de `0.94`, salvo que el operador lo cambie.
