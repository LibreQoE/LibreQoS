# Receta: WISP/FISP con Integracion CRM/NMS Incorporada

Use este patron cuando su operacion este centrada en un CRM/NMS soportado (UISP, Splynx, Netzur, VISP, WISPGate, Powercode o Sonar).

## Ajuste

- Mejor para: cambios recurrentes de abonados, planes gestionados en CRM y automatizacion de ciclo de vida.
- Evitar cuando: su fuente de verdad durable es un pipeline externo propio.

## Prerrequisitos

1. Complete [Quickstart](quickstart-es.md) y pase el health gate.
2. Confirme la integracion en [Integraciones CRM/NMS](integrations-es.md).
3. Confirme propiedad de fuente de verdad en [Modos de Operacion](operating-modes-es.md).

## Implementacion

1. Configure credenciales y parametros en WebUI (`Configuration -> Integrations`).
2. Para despliegues guiados por integracion, mantenga `always_overwrite_network_json = true`.
3. Elija la estrategia de topologia mas liviana que cumpla su necesidad.

| Necesidad | Estrategia sugerida |
|---|---|
| Maximo rendimiento, minima jerarquia | `flat` |
| Jerarquia/control moderado | `ap_only` o `ap_site` |
| Requiere shaping completo de rutas/backhaul | `full` |

4. Habilite refresco recurrente en `/etc/lqos.conf` (por ejemplo `enable_uisp = true`, `enable_splynx = true`, `enable_netzur = true`; use la bandera que corresponda a su integracion).
5. Reinicie scheduler y verifique sincronizacion:

```bash
sudo systemctl restart lqos_scheduler
sudo systemctl status lqos_scheduler
journalctl -u lqos_scheduler --since "15 minutes ago"
```

## Ilustracion de Flujo de Datos

```{mermaid}
flowchart LR
    CRM[CRM/NMS]
    INT[Proceso de Integracion]
    SD[ShapedDevices.csv]
    NJ[network.json]
    SCH[lqos_scheduler]
    LQD[lqosd]
    UI[Estado en WebUI]
    MAN[Ediciones manuales sobre archivos generados]

    CRM --> INT
    INT --> SD
    INT --> NJ
    SD --> SCH
    NJ --> SCH
    SCH --> LQD
    SCH --> UI
    MAN -. Puede sobrescribirse en el siguiente sync .-> SD
    MAN -. Puede sobrescribirse en el siguiente sync .-> NJ
```

## Checklist de Validacion

1. `ShapedDevices.csv` se regenera como se espera tras cada sync.
2. El comportamiento de `network.json` coincide con la politica de overwrite.
3. La salud de vistas en WebUI es correcta.
4. Revise `Scheduler Status` y `Urgent Issues`.
5. Revise `Network Tree Overview` y `Flow Map`.
6. La colocacion de padres y distribucion de colas se ve estable.

## Fallas Comunes

- Conflicto entre ediciones manuales e integracion.
- Profundidad topologica excesiva y presion de CPU.
- Advertencias no detectadas de circuitos sin parent.

## Rollback

1. Vuelva a la estrategia previa estable.
2. Restaure backups de shaping si aplica.
3. Reinicie `lqos_scheduler` y `lqosd`.
4. Confirme que urgencias se limpian y vistas repueblan.

## Paginas Relacionadas

- [Integraciones CRM/NMS](integrations-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
- [Referencia de Configuracion Avanzada](configuration-advanced-es.md)
- [Troubleshooting](troubleshooting-es.md)
