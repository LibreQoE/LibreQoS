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
