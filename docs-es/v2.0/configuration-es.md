# Configurar LibreQoS

## Configuración inicial mediante la herramienta de instalación (desde el .deb)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

La herramienta de instalación configura puente, interfaces, ancho de banda, rangos IP y usuarios de WebUI.

Notas:
- La herramienta se controla con teclado (`Enter` para seleccionar, `Q` para salir sin guardar).
- Si necesita volver a abrirla:
  ```
  sudo apt remove libreqos
  sudo apt install ./{deb_url_v1_5}
  ```

### Próximos pasos

Después de instalar, ingrese a la WebUI en `http://tu_ip_del_shaper:9123`.

Para la mayoría de operadores:
1. Elegir modo de operación: [Modos de operación y fuente de verdad](operating-modes-es.md)
2. Configurar integración en WebUI: [Integraciones CRM/NMS](integrations-es.md)
3. Validar scheduler y shaping en WebUI: [LibreQoS WebUI (Node Manager)](node-manager-ui-es.md)

## Configuración mediante la interfaz web

La mayoría de cambios operativos diarios se realizan en la WebUI (`http://tu_ip_del_shaper:9123/config_general.html`).

### Dónde en la WebUI

- Ajustes generales: `Configuration -> General`
- Ajustes de integración: `Configuration -> Integrations`
- Editor de dispositivos regulados: `Configuration -> Shaped Devices`
- Validación operativa: páginas de `WebUI (Node Manager)` (dashboard/tree/flow/scheduler)

## Modos de operación y fuente de verdad

Lea esto primero antes de cambios en producción:
- [Modos de operación y fuente de verdad](operating-modes-es.md)

## Referencia avanzada de configuración

La configuración por CLI, edición directa de archivos y referencia detallada se movió aquí:
- [Referencia avanzada de configuración](configuration-advanced-es.md)

## Configuración por línea de comando

Esta sección se movió a [Referencia avanzada de configuración](configuration-advanced-es.md#configuración-por-línea-de-comando).

## Jerarquía de red

Esta sección se movió a [Referencia avanzada de configuración](configuration-advanced-es.md#jerarquía-de-red).

## Circuitos

Esta sección se movió a [Referencia avanzada de configuración](configuration-advanced-es.md#circuitos).

## Páginas relacionadas

- [Quickstart](quickstart-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)
