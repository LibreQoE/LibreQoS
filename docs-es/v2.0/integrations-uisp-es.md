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

- La sincronización actualiza automáticamente los datos importados y de shaping que LibreQoS usa con UISP.
- `network.json` queda para despliegues DIY o manuales.
- Use la WebUI para confirmar que la importación y la profundidad del árbol son las esperadas.
- En modo integración, trate ediciones de archivos como temporales.

## Validación en 5 minutos después de cambios UISP

1. Ejecute integración una vez:
```bash
cd /opt/libreqos/src
sudo /opt/libreqos/src/bin/uisp_integration
```
2. Confirme archivos generados/actualizados:
```bash
ls -lh /opt/libreqos/src/topology_import.json /opt/libreqos/src/shaping_inputs.json
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

Las compilaciones UISP actuales también reutilizan ese mismo multiplicador conservador para límites de transporte en adjuntos de infraestructura cuando `infrastructure_transport_caps_enabled = true`. Para esos límites de infraestructura, LibreQoS prefiere la interfaz Ethernet/SFP de transporte activa con mayor velocidad reportada por UISP, con fallbacks exactos por modelo para techos de hardware conocidos como AF60-LR.

La compactación de runtime/exportación para UISP ahora siempre se aplica después de que Topology Manager resuelve la preferencia de adjuntos. `do_not_squash_sites` sigue permitiendo excluir nombres de sitios concretos de esa compactación.

Nota heredada:
- Los valores existentes de `enable_squashing` en `/etc/lqos.conf` se ignoran por compatibilidad hacia atrás.

Las versiones actuales también manejan de forma específica los AP AirMax donde UISP reporta `identification.type == "airMax"` y `identification.role == "ap"`. En esos AP AirMax, `theoreticalTotalCapacity` se usa solo como pista de flexible framing. La velocidad real de shaping sale de `totalCapacity` cuando UISP lo entrega, o de la capacidad direccional más fuerte cuando no lo hace, y la división sigue prefiriendo `dlRatio` cuando UISP lo reporta; si no, usa `airmax_flexible_frame_download_ratio`, cuyo valor por defecto `0.8` significa 80/20 descarga/subida.

Los sondeos de salud de adjuntos en Topology Manager usan las IPs de gestión reportadas por UISP para el par de adjuntos seleccionado. Las compilaciones actuales ya no filtran esas IPs de sondeo mediante `allow_subnets` de shaping; la lista permitida de direcciones de shaping sigue aplicándose a los datos generados de shaping de suscriptores/dispositivos, pero no a los destinos de sondeo de topología del plano de gestión.

Los overrides de tasa por adjunto en Topology Manager permanecen deshabilitados para los adjuntos UISP cuya velocidad proviene directamente de telemetría dinámica de capacidad de radio, por ejemplo cuando UISP está reportando capacidad direccional en vivo. Los adjuntos estáticos de UISP, los casos black-box/fallback y los grupos manuales sí siguen siendo elegibles para overrides de tasa por adjunto.

Las compilaciones actuales de UISP también clasifican el rol de alimentación de cada adjunto para Topology Manager y la exportación runtime. Los roles típicos son `PtP Backhaul`, `PtMP Uplink` y `Wired Uplink`. La compactación runtime/export solo colapsa roles efectivos de tipo backhaul; los APs PtMP de acceso/uplink permanecen visibles en `tree.html`.
