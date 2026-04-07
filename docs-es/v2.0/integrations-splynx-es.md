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
- `always_overwrite_network_json = true` para despliegues guiados por integración

Después, ejecute una sincronización manual y valide salidas antes de aumentar frecuencia de refresh.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` depende de `always_overwrite_network_json`.
- Recomendado: mantener `always_overwrite_network_json = true` para alinear topología con Splynx en cada ciclo.
- Use WebUI para ajustes operativos diarios.

## Validación en 5 minutos después de cambios Splynx

1. Ejecute prueba de integración:
```bash
python3 integrationSplynx.py
```
2. Confirme archivos actualizados:
```bash
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
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

Las versiones actuales también exponen una política compartida de margen para puertos Ethernet en `Configuration -> Integrations -> Integration Defaults`. Las integraciones que pueden aportar la velocidad Ethernet negociada hacia el suscriptor usan un multiplicador conservador por defecto de `0.94`, salvo que el operador lo sobrescriba.
