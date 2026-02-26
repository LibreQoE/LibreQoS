# Escalado y diseño de topología

Esta guía se centra en diseñar `network.json` y elegir estrategias de integración para mantener rendimiento estable a escala.

## Principios de diseño

- Mantenga la jerarquía tan simple como sea posible.
- Evite concentrar demasiado tráfico bajo un único padre de nivel superior.
- Use nombres y relaciones padre/hijo estables para reducir churn de colas.
- Valide cambios de topología en ventana de mantenimiento.

## Selección de estrategia por escala

Use la estrategia más simple que cumpla el objetivo operativo:

```{mermaid}
flowchart TD
    A[¿Necesita visibilidad/control jerárquico?] -->|No| B[flat]
    A -->|Sí| C{¿Necesita agregación por sitio?}
    C -->|No| D[ap_only]
    C -->|Sí| E{¿Necesita shaping de ruta/backhaul completo?}
    E -->|No| F[ap_site]
    E -->|Sí| G[full]
    G --> H{¿Saturación de un solo núcleo?}
    H -->|Sí| I[Usar promote_to_root]
    H -->|No| J[Mantener estrategia full]
```

| Estrategia | Ajuste típico de escala | Tradeoff |
|---|---|---|
| `flat` | Máximo rendimiento, mínima jerarquía | Menor visibilidad/agregación |
| `ap_only` | Redes grandes centradas en APs | Buen rendimiento, visibilidad media |
| `ap_site` | Redes medianas/grandes con sitio+AP | Mejor agregación, costo moderado |
| `full` | Necesidad de ruta/backhaul completo | Mayor control y mayor costo CPU/memoria |

Si necesita `full` y detecta saturación de un solo núcleo, use `promote_to_root`.

## Distribución padre/hijo

Objetivos:

- Balancear padres de nivel superior en tráfico y cantidad de circuitos.
- Evitar ramas excesivamente profundas sin valor real para shaping.
- Mantener nombres de hermanos únicos y estables.

Señales de alerta:

- Un núcleo saturado de forma persistente y el resto ocioso.
- Reconstrucciones grandes de colas por cambios menores.
- CPU Tree de WebUI con distribución muy sesgada.

## Nodos virtuales y agrupación lógica

Los nodos virtuales ayudan para organización y agregación visual.

- Úselos para mejorar operabilidad y reportes.
- No los use para ocultar una topología física confusa.
- Valide colisiones de nombres tras promoción virtual.

## Guardrails de colas/clasificadores

A gran escala, la presión sobre identificadores de clase puede ser real.

- Monitoree eventos urgentes como `TC_U16_OVERFLOW`.
- Si aparece, reduzca complejidad topológica y/o aumente paralelismo de colas.
- Reevalúe profundidad de estrategia (`full` -> `ap_site`/`ap_only`) cuando haya riesgo.

Consulte [Solución de Problemas](troubleshooting-es.md#códigos-de-problemas-urgentes-y-primeras-acciones).

## Checklist de despliegue para cambios de topología

1. Respaldar `network.json` y `ShapedDevices.csv`.
2. Aplicar un conjunto de cambios por vez.
3. Revisar logs de `lqos_scheduler` y `lqosd` tras cada cambio.
4. Validar en WebUI:
   - CPU Tree / CPU Weights
   - comportamiento de mapa de flujos/ASN/árbol
   - estado del scheduler y problemas urgentes
5. Mantener plan de rollback con configuración y archivos previos.

## Páginas relacionadas

- [Integraciones](integrations-es.md)
- [Ajuste de rendimiento](performance-tuning-es.md)
- [StormGuard](stormguard-es.md)
- [Alta Disponibilidad y Dominios de Falla](high-availability-es.md)
- [Configuración](configuration-es.md)
- [Solución de Problemas](troubleshooting-es.md)
