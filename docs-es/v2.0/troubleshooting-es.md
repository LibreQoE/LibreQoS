# Solución de Problemas

## Problemas Comunes

### La contraseña de usuario no funciona

Eliminar el archivo lqusers:
```
sudo rm /opt/libreqos/src/lqusers.toml
sudo systemctl restart lqosd lqos_scheduler
```
Entonces visita: BOX_IP:9123/index.html
Esto le permitirá configurar el usuario nuevamente desde cero utilizando la interfaz web.

### No hay WebUI en x.x.x.x:9123

La interfaz web (WebUI) está controlada por el servicio lqosd. Generalmente, cuando la WebUI no se inicia, se debe a que lqosd está en un estado fallido. Verifica si el servicio lqosd está corriendo con el siguiente comando:
```
sudo systemctl status lqosd
```

Si el estado es 'failed', examina la causa usando journalctl, que muestra el estado completo del servicio:
```
journalctl -u lqosd --since "10 minutes ago"
```
Presiona la tecla End en el teclado para ir al final del registro y ver las últimas actualizaciones.

Lqosd proporcionará razones específicas del fallo, como por ejemplo que una interfaz no está activa, o que le falta soporte para múltiples filas (multi-queue), u otros problemas.

### LibreQoS está en ejecución, pero no se está aplicando el shaping al tráfico

En el archivo /etc/lqos.conf, asegúrese de que los parámetros `to_internet` y `to_network` estén configurados correctamente.
Si no lo están, simplemente intercambie las interfaces y reinicie lqosd y el scheduler.

```
sudo systemctl restart lqosd lqos_scheduler
```

Asegurese de que los servicios estén funcionando correctamente

```
sudo systemctl status lqosd lqos_scheduler
```

El servicio lqos_scheduler depende de que el servicio lqosd esté en buen estado y en ejecución.

### El servicio lqosd no se está ejecutando o falló al iniciar

Verifica el estado del servicio lqosd:
```
sudo systemctl status lqosd
```

Si el estado es 'failed', examine la causa con journalctl, que muestra el estado completo del servicio:
```
journalctl -u lqosd --since "10 minutes ago"
```
Presiona la tecla End en el teclado para ir al final del registro y ver las últimas actualizaciones.

Lqosd le mostrará razones específicas del fallo, como por ejemplo que una interfaz no está activa, que le falta soporte multi-queue, u otros problemas.

### Depuración avanzada de lqosd

Desde la línea de comandos, ejecuta:
```
sudo RUST_LOG=info /opt/libreqos/src/bin/lqosd
```
Esto ejecutará lqosd en modo debug y te proporcionará detalles sobre por qué falló al iniciar.

### RTNETLINK answers: Invalid argument

Este error suele aparecer cuando el qdisc tipo MQ no puede ser añadido correctamente a la interfaz de red (NIC).
Esto sugiere que la tarjeta de red no tiene suficientes filas RX/TX. Asegurese de estar utilizando las [NICs recomendadas](requirements-es.md).

### Error de Python ModuleNotFoundError en Ubuntu 24.04
```
pip uninstall binpacking --break-system-packages --yes
sudo pip uninstall binpacking --break-system-packages --yes
sudo pip install binpacking --break-system-packages
pip uninstall apscheduler --break-system-packages --yes
sudo pip uninstall apscheduler --break-system-packages --yes
sudo pip install apscheduler --break-system-packages
pip uninstall deepdiff --break-system-packages --yes
sudo pip uninstall deepdiff --break-system-packages --yes
sudo pip install deepdiff --break-system-packages
```
### Todas las IPs de clientes aparecen como "Unknown IPs" en lugar de "Shaped Devices" en la interfaz gráfica
```
cd /opt/libreqos/src
sudo systemctl stop lqos_scheduler
sudo python3 LibreQoS.py
```
El texto que se genera en la terminal al ejecutar LibreQoS.py proporciona más detalles sobre errores más específicos relacionados con problemas en los archivos ShapedDevices.csv y network.json.
Una vez que haya identificado el error y corregido ShapedDevices.csv y/o network.json, ejecute:

```sudo systemctl start lqos_scheduler```

### Colisión al promover nodos virtuales (network.json)

Si LibreQoS.py falla con un error como `Virtual node promotion collision: 'AP_A' already exists at this level.`, tienes un nodo con `"virtual": true` cuyos hijos se promueven a un nivel padre donde ya existe un nodo con el mismo nombre.

Renombra uno de los nodos en conflicto (los nombres deben ser únicos entre hermanos después de la promoción), o reestructura la jerarquía para que los hijos promovidos no colisionen.

### Error de segmentación (segfault) en systemd

Si experimenta un segfault en systemd, este es un problema conocido en systemd [1](https://github.com/systemd/systemd/issues/36031) [2](https://github.com/systemd/systemd/issues/33643).
Como solución temporal, puede compilar systemd desde cero:

### Instalar dependencias de compilación

```
sudo apt update
sudo apt install build-essential git meson libcap-dev libmount-dev libseccomp-dev \
libblkid-dev libacl1-dev libattr1-dev libcryptsetup-dev libaudit-dev \
libpam0g-dev libselinux1-dev libzstd-dev libcurl4-openssl-dev
```

#### Clonar el repositorio de systemd desde github

```
git clone https://github.com/systemd/systemd.git
cd systemd
git checkout v257.5
meson setup build
meson compile -C build
sudo meson install -C build
```
Después, reinicia el sistema y confirma la versión de systemd con `systemctl --version`

```
libreqos@libreqos:~$ systemctl --version
systemd 257 (257.5)
```
