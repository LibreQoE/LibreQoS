# Integraciones CRM/NMS

**Versión: LibreQoS v3.** Cómo LibreQoS v3 importa abonados y topología
desde su CRM o NMS.

Las integraciones funcionan igual que en v2.0. Ahora se ejecutan dentro del
servicio `libreqos-scheduler`, que arranca después de `lqosd`, y el compilador
integrado convierte lo importado en la configuración de shaping del bridge.

```{mermaid}
flowchart LR
    A[CRM/NMS] --> B[Tarea de integración en libreqos-scheduler]
    B --> H[topology_import.json]
    H --> C[compilador de lqosd]
    C --> D[bridge DPDK]
    C --> G[estado en la WebUI]
```

## Elija su fuente de verdad

| Camino | Ideal para | Dónde van los cambios permanentes de shaping |
|---|---|---|
| Integración incluida | Operadores con un CRM/NMS soportado | El CRM/NMS, a través de la integración |
| Sus propios archivos | Operadores con scripts de sincronización propios | `network.json` y `ShapedDevices.csv`, que usted genera |

Con una integración incluida activa, las ediciones directas de los datos
importados pueden reemplazarse en la siguiente actualización. Si mantiene sus
propios archivos, siga generando `network.json` y `ShapedDevices.csv` como en
v2.0.

## Integraciones incluidas

- [Splynx](integrations-splynx-es.md)
- [UISP](integrations-uisp-es.md)
- [Netzur](integrations-netzur-es.md)
- [VISP](integrations-visp-es.md)
- [WISPGate](integrations-wispgate-es.md)
- [Powercode](integrations-powercode-es.md)
- [Sonar](integrations-sonar-es.md)

```{toctree}
:hidden:

integrations-splynx-es
integrations-uisp-es
integrations-netzur-es
integrations-visp-es
integrations-wispgate-es
integrations-powercode-es
integrations-sonar-es
```

Para redes PPPoE o DHCP-RADIUS donde los abonados aparecen y desaparecen,
consulte [Contabilidad RADIUS y circuitos dinámicos](radius-es.md).

## Dónde se configuran

- Ajustes: en la WebUI, **Configuration → Data Sources** elige la fuente de
  topología y los valores compartidos de integración, y cada integración tiene
  su propia página en **Configuration**. Los mismos ajustes están en
  `/etc/libreqos/lqos.conf`.
- Intervalo de actualización: `queue_refresh_interval_mins`.
- Después de editar `lqos.conf` a mano:
  `sudo systemctl restart libreqos-scheduler`.
- Estado después de un cambio: las vistas **Scheduler** y **Alerts** de la
  WebUI, o `journalctl -u libreqos-scheduler --since "30 minutes ago"`.

Los datos importados se guardan en
`/opt/libreqos/state/topology/topology_import.json` y
`/opt/libreqos/state/shaping/shaping_inputs.json`.

## Modos de topología

Cada integración admite un conjunto de valores de `topology.compile_mode`:

- UISP: `flat`, `ap_only`, `ap_site`, `full`
- Splynx: `flat`, `ap_only`, `ap_site`, `full`
- Sonar: `flat`, `full`

`ap_site` es el valor recomendado. En v3 el modo solo cambia la forma del árbol:
el bridge maneja árboles profundos de forma nativa, reparte los sitios y APs
entre sus workers por sí mismo, y los nodos virtuales ya no existen (un nodo
marcado `virtual` se procesa como un sitio real). `promote_to_root` se sigue
aceptando, pero ya no hace falta para repartir la carga de CPU.

## IDs de nodo

Los nodos de `network.json` pueden llevar un campo opcional `"id"`. Si existe,
LibreQoS lo usa para asociar los ajustes de ancho de banda guardados por sitio;
la coincidencia por nombre sigue como respaldo, así que los nombres deben seguir
siendo únicos.

| Integración | IDs de nodo | Notas |
|---|---|---|
| UISP | Sí | Sitios y dispositivos reales; los nodos sintéticos reciben IDs estables generados. |
| Splynx | Sí | Network sites y nodos AP/sitio. |
| Sonar | Sí | Nodos de sitio y AP. |
| Netzur | Parcial | Solo cuando los datos de zona tienen un ID de zona estable. |
| VISP | Parcial | Cuando las relaciones IRM de VISP están completas; si no, conexión plana. |
| Powercode | No | No genera nodos de topología. |
| WISPGate | No | No genera nodos de topología. |

## Velocidades de cliente

Para las integraciones que importan velocidades de plan sin ajustar, LibreQoS
fija la velocidad máxima de cada circuito en
`max(velocidad_plan × bandwidth_overhead_factor, velocidad_plan × client_bandwidth_multiplier)`.
Las integraciones que ya importan velocidades efectivas las conservan tal cual.

Los archivos heredados `integrationUISPbandwidths.csv` e
`integrationSplynxBandwidths.csv` se migran a entradas `AdjustSiteSpeed` en
`lqos_overrides.json` y se renombran a `.backup`.
