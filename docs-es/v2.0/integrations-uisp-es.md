# Integración con UISP

## Resumen

Use esta integración cuando UISP sea su fuente de verdad CRM/NMS.

## Configuración básica

1. Configure UISP en `/etc/lqos.conf`.
2. Elija estrategia de topología y estrategia de suspensión.
3. Habilite sincronización automática y reinicie scheduler.

## Selector rápido para nuevos operadores

Use esta guía para evitar confusión con opciones:

1. Si está empezando, use `strategy = "ap_only"`.
2. Cambie a `ap_site` cuando necesite agregación explícita por sitio.
3. Use `full` cuando necesite jerarquía/backhaul completo, tenga margen de CPU y ya haya validado topología/overrides.
4. Use `flat` solo si la jerarquía no es necesaria y prioriza rendimiento máximo.

## Expectativas en router mode

- Router mode en UISP es compatible, pero depende de que la topología en UISP esté bien definida.
- LibreQoS se centra en shaping/jerarquía de colas, no en enforcement completo del ciclo de vida del suscriptor.
- La suspensión de cuentas normalmente se aplica en edge/BNG/autenticación; `suspended_strategy` define solo el comportamiento de shaping en LibreQoS.

## Notas operativas

- `ShapedDevices.csv` se regenera en cada sincronización.
- `network.json` depende de `always_overwrite_network_json`.
- Para despliegues guiados por integración, use `always_overwrite_network_json = true` para mantener la topología alineada con UISP en cada ciclo.
- En modo integración, trate ediciones de archivos como temporales.

## Validación en 5 minutos después de cambios UISP

1. Ejecute integración una vez:
```bash
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```
2. Confirme archivos generados/actualizados:
```bash
ls -lh /opt/libreqos/src/network.json /opt/libreqos/src/ShapedDevices.csv
```
3. Verifique salud de servicios:
```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqos_scheduler --since "30 minutes ago"
```
4. Valide en WebUI:
- Scheduler Status saludable
- Profundidad de árbol acorde a la estrategia elegida

## Referencia completa

- [Referencia detallada de UISP](integrations-reference-es.md#integración-con-uisp)
- [Modos de operación y fuente de verdad](operating-modes-es.md)

Los valores por defecto de integración también incluyen el límite compartido para puertos Ethernet. Cuando UISP puede detectar la velocidad Ethernet negociada hacia el suscriptor, las versiones actuales aplican un multiplicador conservador por defecto de `0.94`, salvo que el operador lo sobrescriba en `Configuration -> Integrations -> Integration Defaults`.

Las versiones actuales también manejan de forma específica los AP AirMax donde UISP reporta `identification.type == "airMax"` y `identification.role == "ap"`. En esos AP AirMax, `theoreticalTotalCapacity` se usa solo como pista de flexible framing. La velocidad real de shaping sale de `totalCapacity` cuando UISP lo entrega, o de la capacidad direccional más fuerte cuando no lo hace, y la división sigue prefiriendo `dlRatio` cuando UISP lo reporta; si no, usa `airmax_flexible_frame_download_ratio`, cuyo valor por defecto `0.8` significa 80/20 descarga/subida.
