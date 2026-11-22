# Integración con Splynx

**Versión: LibreQoS v3.**

## Resumen

Use esta integración cuando Splynx sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure Splynx en `/etc/libreqos/lqos.conf`.
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
- Cuando Splynx no puede inferir el padre de un circuito desde `access_device` o los metadatos del router, LibreQoS deja ese circuito sin padre para shaping en lugar de adjuntarlo a un sitio o AP sintético. Corrija el padre en Splynx o Topology Manager si debe aparecer bajo un sitio o AP real.
- Si dos filas de hardware de monitoreo de Splynx tienen el mismo título, LibreQoS agrega el ID de hardware de Splynx a esos nombres duplicados de nodos de topología. Por ejemplo, dos AP llamados `Tower South` aparecen como `Tower South (1122)` y `Tower South (1980)`.
- En modo `ap_site`, LibreQoS trata los Network Sites de Splynx como contenedores de sitio y las filas de monitoreo con `access_device = 1` como AP o nodos de acceso.

## Validación en 5 minutos después de cambios Splynx

1. Ejecute prueba de integración:
```bash
sudo /opt/libreqos/venv/bin/python integrationSplynx.py
```
2. Confirme archivos actualizados:
```bash
sudo ls -lh /opt/libreqos/state/topology/topology_import.json /opt/libreqos/state/shaping/shaping_inputs.json
```
3. Confirme salud de servicios:
```bash
sudo systemctl status libreqos-lqosd libreqos-scheduler
journalctl -u libreqos-scheduler --since "30 minutes ago"
```
4. Verifique en WebUI que la vista Scheduler y profundidad de árbol coincidan con la estrategia elegida.

## Referencia completa

- [Guía completa de Splynx (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/integrations-splynx.html)
- [Operación de LibreQoS v3 (en inglés)](https://libreqos.readthedocs.io/en/latest/docs/v3/operate.html)

Las versiones actuales también exponen una política compartida de margen para puertos Ethernet en `Configuration -> Data Sources -> Integration Defaults`. Las integraciones que pueden aportar la velocidad Ethernet negociada hacia el suscriptor usan un multiplicador conservador por defecto de `0.94`, salvo que el operador lo cambie.
