# Actualizar 1.4 a la última versión

```{advertencia}
Si utiliza el puente XDP, el tráfico dejará de pasar a través del puente durante la actualización (el puente XDP solo funciona mientras lqosd se esta ejecutando).
```

## Si instalaste con Git

1. Cambie a su directorio LibreQoS (ejemplo: cd /opt/LibreQoS)
2. Actualize desde Git: `git pull`
3. Recompile: `./build-rust.sh`
4. `sudo rust/remove_pinned_maps.sh`

Ejecute los siguientes comandos para reiniciar los servicios de LibreQoS:

```shell
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
sudo systemctl restart lqos_scheduler
```

## Si instalaste mediante el repositorio APT

En este caso, lo único que necesita hacer es ejecutar `sudo apt update && sudo apt upgrade` y LibreQoS debería instalar el nuevo paquete.
