# Actualización a la última versión

```{warning}
Si usa el puente XDP, el tráfico dejará de pasar brevemente a través del puente cuando se reinicie lqosd (el puente XDP solo funciona mientras se ejecuta lqosd).
```

## Si instaló el .deb

Descargue el último .deb desde [libreqos.io/#download](https://libreqos.io/#download).

Descomprima el archivo .zip y transfiera el .deb a su caja LibreQoS, instalándolo con:
```
sudo apt install ./[deb file name]
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
