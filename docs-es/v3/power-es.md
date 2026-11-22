# Perfil de energía de la CPU

**Versión: LibreQoS v3.** Cómo LibreQoS fija la política de energía de la
CPU del host y cómo cambiarla.

Los workers de DPDK sondean sus colas de la NIC sin pausa, así que la
frecuencia de la CPU limita directamente cuántos paquetes puede procesar cada
worker. LibreQoS administra la política de energía de la CPU mediante un perfil
de energía, definido en `/etc/libreqos/lqos.conf`:

```toml
[power]
profile = "balanced"     # balanced (predeterminado) | eco | performance
eco_max_freq_pct = 65    # solo eco: límite como porcentaje del máximo sin turbo
```

## Los perfiles

Todos los perfiles **desactivan el turbo**. LibreQoS no ofrece una forma de
volver a activarlo.

| Perfil | Qué cambia | Cuándo usarlo |
|---|---|---|
| `balanced` (predeterminado) | Turbo desactivado; nada más. | Casi todos los hosts. |
| `eco` | Además, el governor `powersave`, la preferencia de energía `power` y un límite de frecuencia de `eco_max_freq_pct` (65 % por defecto) del máximo sin turbo. | Hosts con poca carga donde importan más el consumo o el calor que la capacidad. |
| `performance` | Además, el governor y la preferencia de energía `performance`, y solo estados de reposo poco profundos. Sigue limitado al máximo sin turbo. | Hosts que trabajan cerca de su límite de paquetes por segundo. |

En una plataforma sin interruptor de turbo, "turbo desactivado" significa
limitar la CPU a su frecuencia nominal (sin boost).

`eco` reduce la capacidad. En el [host de referencia](requirements-es.md), con
40 Gbps de tráfico similar al de producción, `eco` (2,9 GHz) perdió el 16,6 %
del tráfico de bajada, mientras que `balanced` (4,5 GHz) perdió el 0,03 %.

## Cambiar el perfil

```bash
sudo libreqos-power-profile status              # perfil configurado y plataforma
sudo libreqos-power-profile set performance     # escribe [power] profile y lo aplica ya
sudo libreqos-power-profile apply               # vuelve a aplicar el perfil de lqos.conf
sudo libreqos-power-profile restore             # restaura los valores previos a LibreQoS
```

Agregue `--dry-run` para ver qué cambiaría sin escribir nada. El cambio no
interrumpe el shaping.

El servicio `libreqos-power` aplica el perfil en cada arranque. Desinstalar el
paquete `libreqos` restaura los valores originales.

## A tener en cuenta

- **Los valores originales se registran una sola vez**, en
  `/var/lib/libreqos/power.orig`, la primera vez que se aplica un perfil.
  `restore` vuelve a esos valores, incluso después de varios cambios de perfil.
- **Otras herramientas de energía entran en conflicto.** Si `tuned`, `cpupower`
  o `power-profiles-daemon` están activos, `libreqos-power-profile` se niega a
  ejecutarse, porque esas herramientas reescriben los mismos ajustes.
  Desactívelas o use `--force`. Después de aplicar, la herramienta vuelve a leer
  cada ajuste y avisa de los que no se mantuvieron.
- **La clave de v2 `[tuning] set_cpu_governor_performance`** se ignora en v3;
  `[power]` controla los ajustes de la CPU. `lqosd` registra una advertencia si
  la clave sigue en `true`.
- **En una VM**, el invitado no puede fijar la política de CPU del host. La
  herramienta muestra los pasos que hay que ejecutar en el hipervisor.
