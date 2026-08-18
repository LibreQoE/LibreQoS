# Ajuste de rendimiento

## Triage por síntoma

| Síntoma | Primeros checks | Siguiente acción probable |
|---|---|---|
| Throughput menor al esperado | distribución de colas/CPU, profundidad de estrategia, salud del scheduler | reducir profundidad o usar `promote_to_root` y volver a medir |
| CPU alta en un solo núcleo | desbalance IRQ/colas, cuello en nodo raíz | rebalancear afinidad y/o promover sitios pesados a raíz |
| Cambio de rendimiento tras cambios de integración | cambio no intencional de estrategia/profundidad | confirmar estrategia (`flat`/`ap_only`/`ap_site`/`full`) y validar árbol/flujo |

## Base de CPU e IRQ

LibreQoS ahora intenta configurar automáticamente el governor de CPU en modo `performance` durante el ajuste de arranque en hosts bare metal e hipervisor.

Este comportamiento está habilitado por defecto mediante `[tuning].set_cpu_governor_performance = true`. Desactívelo solo si su plataforma requiere otra política de governor.

Si necesita verificar manualmente el governor actual:

```bash
cpupower frequency-info | grep 'current policy'
```

Confirme que el conteo de colas NIC y la distribución por CPU sean razonables:

```bash
ethtool -l <interfaz>
grep -E 'CPU\\(|IRQ' /proc/interrupts | head -n 50
```

Si un solo núcleo/cola está saturado y los demás están ociosos, rebalancee afinidad IRQ/colas y revise la configuración de colas en `/etc/lqos.conf`.

## Presión por topología/colas

Cuando modele jerarquías grandes:

- Prefiera estrategias de menor profundidad (`ap_only`/`ap_site`) salvo que necesite jerarquía completa.
- Use `promote_to_root` en topologías multi-sitio para evitar cuellos de botella de un solo núcleo.
- Valide CPU Tree/CPU Weights en WebUI después de cambios topológicos importantes.

Consulte también [Escalado y diseño de topología](scale-topology-es.md).

## Frecuencia del scheduler y carga

La cadencia de `lqos_scheduler` afecta carga del plano de control y velocidad de cambios.

- Comience con `queue_refresh_interval_mins` conservador.
- Reduzca intervalo solo si su integración/API y host soportan ese churn.
- Tras cambiar intervalo, monitoree:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - estado del scheduler en WebUI

## Despliegue de StormGuard

Si usa StormGuard:

1. Inicie con `dry_run = true`.
2. Observe al menos un periodo de alto tráfico.
3. Pase a modo activo solo tras revisar estado/depuración.

Vea [StormGuard](stormguard-es.md).

## Arranque lento / dependencias de red

Si Ubuntu arranca lento por dependencias de `network-online`, inspeccione:

```bash
systemctl show -p WantedBy network-online.target
```

En algunos entornos Ubuntu, deshabilitar servicios no usados de cloud/iSCSI puede ayudar:

```bash
sudo systemctl disable cloud-config iscsid cloud-final
```

Valide esta acción contra su entorno antes de aplicarla.

## Convergencia de ruteo (OSPF)

Para despliegues enrutados, ajuste timers OSPF entre core y edge para reducir ventanas de indisponibilidad en reinicios:

- hello interval
- dead interval

## Páginas relacionadas de optimización y resiliencia

- [Escalado y diseño de topología](scale-topology-es.md)
- [StormGuard](stormguard-es.md)
- [Alta Disponibilidad y Dominios de Falla](high-availability-es.md)
- [Solución de problemas](troubleshooting-es.md)
