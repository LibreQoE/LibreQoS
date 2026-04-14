# Configurar LibreQoS

## Propósito de esta página

Use esta página para operaciones diarias y configuración mediante la WebUI.

Use [Inicio rápido](quickstart-es.md) para la ruta de instalación y despliegue del día 1.
Use [Referencia avanzada de configuración](configuration-advanced-es.md) para edición directa de archivos y flujos centrados en CLI.

## Configuración inicial en la WebUI

Las instalaciones actuales usan la WebUI principal de LibreQoS en el puerto `9123` para el proceso inicial.

Después de instalar el paquete:
1. Abra `http://tu_ip_del_shaper:9123`
2. Cree el primer usuario administrador si LibreQoS lo redirige a `first-run.html`
3. Inicie sesión
4. Abra `Complete Setup`
5. Elija cómo recibirá LibreQoS los datos de suscriptores y topología

Para la mayoría de los operadores, `Complete Setup` es donde ocurre la decisión importante del inicio:
- Integración incluida para UISP, Splynx, VISP, Netzur, Powercode, Sonar o WispGate
- Importador personalizado si su propio proceso escribe `network.json` y `ShapedDevices.csv`
- Archivos manuales si usted quiere mantener esos archivos directamente

Si Scheduler Status todavía muestra `Setup Required`, LibreQoS aún no está listo para hacer shaping de suscriptores. Termine `Complete Setup` y confirme que la fuente elegida ya publicó datos válidos antes de tratar el sistema como listo para producción.

## Configuración mediante la interfaz web

La mayoría de los cambios operativos diarios se hacen en la WebUI (`http://tu_ip_del_shaper:9123/config_general.html`).

Las versiones actuales usan una disposición coherente en las páginas General, RTT, Queues, TreeGuard, Network Mode, Integration Defaults, Network Layout, Insight, páginas de integración por proveedor, IP Ranges, Flow Tracking y Shaped Devices. Integration Defaults también incluye la política compartida de margen para puertos Ethernet usada por integraciones que pueden detectar la velocidad negociada hacia el suscriptor.

### Dónde en la WebUI

- Ajustes generales: `Configuration -> General`
- Ajustes de integración: `Configuration -> Integrations`
- Editor de topología: `Configuration -> Network Layout`
- Editor de dispositivos regulados: `Configuration -> Shaped Devices`
- Validación operativa en tiempo de ejecución: páginas de `WebUI (Node Manager)` como dashboard, tree, flow y scheduler

Cuando una integración está gestionando sus datos de shaping, los editores `Network Layout` y `Shaped Devices` siguen visibles pero pasan a modo de solo lectura en la WebUI.

## Fuente de verdad

Lea esto primero antes de hacer cambios en producción:
- [Modos de operación y fuente de verdad](operating-modes-es.md)

Los cambios permanentes de shaping deben hacerse en un solo lugar.

Si una integración controla su topología y suscriptores, mantenga allí los cambios permanentes.
Si su propio importador controla los archivos, mantenga allí los cambios permanentes.
Si usa archivos manuales de forma intencional, mantenga los cambios permanentes en `network.json` y `ShapedDevices.csv`.

## Notas importantes

Nota de topología:
- Los nombres de nodo en `network.json` deben ser globalmente únicos en todo el árbol. Los nombres duplicados fallan la validación y no son aceptados por el guardado de la WebUI ni por `LibreQoS.py`.
- Cuando un nodo expone un `id` estable, LibreQoS prefiere ese `id` para overrides guardados de ancho de banda por sitio, manteniendo la coincidencia heredada por nombre como alternativa.

Nota sobre modo de cola:
- Las versiones actuales usan `queue_mode` con los valores `shape` y `observe`. La etiqueta antigua `monitor_only` sigue existiendo solo como alias de compatibilidad.

Nota sobre logo compartido:
- `Configuration -> General` incluye un control opcional y una carga PNG para mostrar un logo del operador junto al logo de LibreQoS.
- LibreQoS guarda el archivo subido como `cobrand.png` en el directorio de assets estáticos en tiempo de ejecución.
- La opción de nivel superior `display_cobrand` en `/etc/lqos.conf` es opcional. Si no está presente, LibreQoS la trata como `false`.
- La barra lateral muestra la imagen compartida con 48px de alto para igualar el logo de LibreQoS, con un ancho máximo de 176px.

## Perfiles QoO (Quality of Outcome) (`qoo_profiles.json`)

LibreQoS muestra QoO como una estimación de la calidad de internet basada en latencia y pérdida.

### Dónde vive el archivo

`<lqos_directory>/qoo_profiles.json`

### Selección de perfil

- WebUI: `Configuration -> General -> QoO Profile`
- Archivo de configuración: defina `qoo_profile_id` en `/etc/lqos.conf`

Ejemplo:

```toml
# /etc/lqos.conf
qoo_profile_id = "web_browsing"
```

### Aplicación de cambios

- Los cambios en `qoo_profiles.json` se aplican automáticamente.
- Si cambia `/etc/lqos.conf`, reinicie `lqosd`.

## ¿Necesita cambios por CLI o por archivos?

Para edición directa de archivos (`/etc/lqos.conf`, `network.json`, `ShapedDevices.csv`), overrides y material de referencia más profundo sobre topología o circuitos, use:

- [Referencia avanzada de configuración](configuration-advanced-es.md)

## Páginas relacionadas

- [Inicio rápido](quickstart-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)
