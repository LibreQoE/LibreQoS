# Receta: Maritimo con WAN Variable y StormGuard

Use este patron cuando la capacidad WAN cambia de forma material en el tiempo (por ejemplo, enlaces satelitales) y necesita ajustes acotados de limites de colas.

## Ajuste

- Mejor para: un barco o dominio WAN de alta variacion representado por un nodo top-level.
- Evitar cuando: pretende administrar decenas o cientos de targets con StormGuard.

## Prerrequisitos

1. Complete [Quickstart](quickstart-es.md).
2. Revise alcance y limites en [StormGuard](stormguard-es.md).
3. Confirme comportamiento de fuente de verdad en [Modos de Operacion](operating-modes-es.md).

## Patron de Topologia

Use un nodo top-level llamado `Ship`, con subnodos debajo.

```json
{
  "Ship": {
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 200,
    "children": {
      "Deck_A": {
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 100
      },
      "Deck_B": {
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 100
      }
    }
  }
}
```

## Configuracion de StormGuard

```toml
[stormguard]
enabled = true
dry_run = true
targets = ["Ship"]
minimum_download_percentage = 0.5
minimum_upload_percentage = 0.5
log_file = "/var/log/stormguard.csv"
```

## Ilustracion de Loop de Control

```{mermaid}
flowchart LR
    METRICS[Metricas del nodo Ship\nthroughput, RTT, contexto de perdida]
    SG[Evaluador StormGuard]
    LIMITS[Ajustes acotados de limites]
    QOE[Calidad observada del enlace]

    METRICS --> SG
    SG --> LIMITS
    LIMITS --> QOE
    QOE --> METRICS
```

## Secuencia de Rollout

1. Inicie con `dry_run = true`.
2. Observe multiples periodos de carga.
3. Confirme que ajustes son razonables y acotados.
4. Cambie a `dry_run = false`.

## Checklist de Validacion

1. Las vistas de StormGuard muestran `Ship` como target activo.
2. Los limites efectivos se ajustan bajo congestion y respetan pisos.
3. Mejora RTT/retransmisiones en periodos estresados.
4. No hay drift de nombres entre target y jerarquia actual.

## Rollback

1. Ponga `[stormguard] enabled = false` (o vuelva a `dry_run = true`).
2. Reinicie servicios:

```bash
sudo systemctl restart lqosd lqos_scheduler
```

3. Verifique estabilidad sin ajustes adaptativos.

## Paginas Relacionadas

- [StormGuard](stormguard-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
- [Troubleshooting](troubleshooting-es.md)
