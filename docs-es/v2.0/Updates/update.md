# Actualización a la última versión

```{warning}
Si usa el puente XDP, el tráfico dejará de pasar brevemente a través del puente cuando se reinicie lqosd (el puente XDP solo funciona mientras se ejecuta lqosd).
```

## Si instaló el .deb

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/libreqos_1.5-RC2.202510052233-1_amd64.deb
sudo apt install ./libreqos_1.5-RC2.202510052233-1_amd64.deb
sudo systemctl restart lqosd lqos_scheduler
```

Ahora reinicie el servidor LibreQoS con:
```
sudo reboot
```
Esto eliminará los mapas eBPF antiguos y cargará la última versión de LibreQoS..

## Si lo instalaste con Git

1. Cambie a su directorio `LibreQoS`(e.g. `cd /opt/LibreQoS`)
2.Actualización desde Git: `git pull`
3. ```git switch develop```
5. Recompile: `./build-rust.sh`
6. `sudo rust/remove_pinned_maps.sh`

Ejecute los siguientes comandos para recargar los servicios de LibreQoS.

```shell
sudo systemctl restart lqosd lqos_scheduler
```
