# Actualización

```{warning}
Si usas el puente XDP, el tráfico dejará de pasar por el puente mientras lqosd se reinicia (el puente XDP solo opera mientras lqosd está en ejecución).
```

## Si instalaste el .deb

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
sudo systemctl restart lqosd lqos_scheduler
```

Ahora reinicia tu servidor LibreQoS con:

```
sudo reboot
```

Esto limpiará los mapas eBPF antiguos y cargará la última versión de LibreQoS.

### Validación obligatoria post-actualización

Después de actualizar/reiniciar, ejecute:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "20 minutes ago"
```

Luego valide:
1. Dashboard y Scheduler Status saludables en WebUI.
2. En modo integración: sincronización reciente con resultados esperados en `ShapedDevices.csv`/`network.json`.
3. Profundidad topológica consistente con la estrategia seleccionada.

Si algo falla, vaya a [Solución de problemas](troubleshooting-es.md) antes de otros cambios.

## Si instalaste desde Git

1. Cambia a tu directorio `LibreQoS` (por ejemplo `cd /opt/LibreQoS`)
2. Actualiza desde Git: `git pull`
3. ```git switch develop```
4. Recompila: `./build-rust.sh`
5. `sudo rust/remove_pinned_maps.sh`

Ejecuta los siguientes comandos para recargar los servicios de LibreQoS:

```shell
sudo systemctl restart lqosd lqos_scheduler
```

## Síntomas para pausar y hacer triage

Detenga el rollout y haga triage si aparece:

- `lqosd` o `lqos_scheduler` no saludable tras reinicio
- cambio inesperado de jerarquía tras sync de integración
- scheduler persistentemente no saludable en WebUI
