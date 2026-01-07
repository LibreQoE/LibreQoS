# Comparte tu antes y después

Te pedimos que compartas una captura anonimizad​a de tu despliegue de LibreQoS antes (modo monitor) y después (colas activadas) en el [LibreQoS Chat](https://chat.libreqos.io/join/fvu3cerayyaumo377xwvpev6/). Esto nos ayuda a medir el impacto del software y, de paso, nos saca una sonrisa.

1. Activa el modo monitor only.
2. Déjalo ejecutándose durante 1 semana.
3. Desactiva el modo monitor only.
4. Activa la opción **Redact** en la WebUI de LTS para ocultar datos sensibles.
5. Toma la captura.

## Activar el modo monitor only

```shell
sudo nano /etc/lqos.conf
```

En la sección `[queues]`, establece `monitor_only = true` para pasar al modo monitor.

Después reinicia lqosd y lqos_scheduler:

```shell
sudo systemctl restart lqosd lqos_scheduler
```

## Desactivar el modo monitor only

```shell
sudo nano /etc/lqos.conf
```

En la sección `[queues]`, establece `monitor_only = false` para salir del modo monitor.

Después reinicia lqosd y lqos_scheduler:

```shell
sudo systemctl restart lqosd lqos_scheduler
```
