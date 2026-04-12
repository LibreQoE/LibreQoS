# Configurar LibreQoS

## Propósito de esta página

Use esta página para operaciones diarias y configuración por WebUI. Use [Referencia avanzada de configuración](configuration-advanced-es.md) para edición directa de archivos y flujos centrados en CLI.

## Configuración inicial mediante la herramienta de instalación (desde el .deb)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

La herramienta de instalación configura puente, interfaces, ancho de banda, rangos IP y usuarios de WebUI.

Notas:
- La herramienta se controla con teclado (`Enter` para seleccionar, `Q` para salir sin guardar).

### Próximos pasos

Después de instalar, ingrese a la WebUI en `http://tu_ip_del_shaper:9123`.

Para la mayoría de operadores:
1. Elegir modo de operación: [Modos de operación y fuente de verdad](operating-modes-es.md)
2. Configurar integración en WebUI: [Integraciones CRM/NMS](integrations-es.md)
3. Validar scheduler y shaping en WebUI: [LibreQoS WebUI (Node Manager)](node-manager-ui-es.md)

## Configuración mediante la interfaz web

La mayoría de cambios operativos diarios se realizan en la WebUI (`http://tu_ip_del_shaper:9123/config_general.html`).

Las versiones actuales usan un diseño coherente de configuración en las páginas General, RTT, Queues, TreeGuard, Network Mode, valores por defecto de integración, Network Layout, Insight, páginas de integración por proveedor, IP Ranges, Flow Tracking y Shaped Devices. Los valores por defecto de integración también incluyen la política compartida de margen para puertos Ethernet usada por integraciones que pueden detectar la velocidad negociada hacia el suscriptor.

### Dónde en la WebUI

- Ajustes generales: `Configuration -> General`
- Ajustes de integración: `Configuration -> Integrations`
- Editor de layout de red: `Configuration -> Network Layout`
- Editor de dispositivos regulados: `Configuration -> Shaped Devices`
- Validación operativa: páginas de `WebUI (Node Manager)` (dashboard/tree/flow/scheduler)

Cuando una integración está gestionando sus datos de shaping, los editores `Network Layout` y `Shaped Devices` siguen siendo visibles pero pasan a modo de solo lectura en la WebUI.

Nota de topología:
- Los nombres de nodo en `network.json` deben ser globalmente únicos en todo el árbol. Los nombres duplicados ahora fallan la validación y no son aceptados por el guardado de la WebUI ni por `LibreQoS.py`.

## Modos de operación y fuente de verdad

Lea esto primero antes de cambios en producción:
- [Modos de operación y fuente de verdad](operating-modes-es.md)

## Perfiles de QoE (`qoo_profiles.json`)

LibreQoS muestra QoE como una estimación de la calidad de Internet basada en latencia y pérdida.

Para la metodología actual de cálculo de RTT/QoE por circuito usada en las vistas de experiencia por circuito de la WebUI, consulte [Metodología de QoE y RTT por circuito](qoe-rtt-methodology-es.md).

### Dónde vive el archivo

`<lqos_directory>/qoo_profiles.json`

### Selección de perfil

- **WebUI**: `Configuration -> General -> QoE Profile`
- **Archivo de configuración**: configure `qoo_profile_id` en `/etc/lqos.conf`

Ejemplo:

```toml
# /etc/lqos.conf
qoo_profile_id = "web_browsing"
```

### Aplicar cambios

- Los cambios a `qoo_profiles.json` se detectan automáticamente.
- Si cambia `/etc/lqos.conf`, reinicie `lqosd`.

## ¿Necesita cambios por CLI o por archivos?

Para edición directa de archivos (`/etc/lqos.conf`, `network.json`, `ShapedDevices.csv`), overrides y material de referencia profundo sobre topología/circuitos, use:

- [Referencia avanzada de configuración](configuration-advanced-es.md)

## Páginas relacionadas

- [Quickstart](quickstart-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Metodología de QoE y RTT por circuito](qoe-rtt-methodology-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)
