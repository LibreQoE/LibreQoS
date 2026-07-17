# Actualización

```{warning}
Si usas el puente XDP, el tráfico dejará de pasar por el puente mientras lqosd se reinicia (el puente XDP solo opera mientras lqosd está en ejecución).
```

## Si instalaste el .deb

```{important}
Desde v2.0, la ingesta de circuitos mapeados depende de un estado válido de licencia/grant con derecho. Sin un estado válido de licencia/grant de Insight o Local, LibreQoS solo lee los primeros 1000 circuitos mapeados válidos al estado de shaping activo. Esto incluye estado local de grant/licencia expirado o inválido por cualquier motivo. Vea [Comportamiento de licenciamiento de Insight](insight-es.md#límites-de-circuitos-mapeados-y-estado-de-licencia) y [Solución de problemas](troubleshooting-es.md).

A partir de v2.2, la API del nodo usa la misma política. Las redes con 1.000 circuitos mapeados válidos o menos pueden usar la API sin una suscripción a Insight; las redes mayores requieren un derecho válido de API o Insight. Las llamadas continúan autenticadas. Un administrador puede crear hasta 16 claves locales con nombre en **License & Services**; cada clave se muestra una sola vez, se guarda únicamente como resumen SHA-256 y se puede revocar. Las claves de licencia y el token local heredado siguen siendo compatibles, y los cambios pueden tardar hasta 30 segundos en surtir efecto.
```

Ejecuta:

```bash
cd /tmp
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

Usar `/tmp` evita problemas de permisos con `.deb` locales cuando `apt` no puede acceder al paquete almacenado en un directorio home privado con el usuario `_apt`.

Si `apt install` termina normalmente, reinicia los servicios:

```bash
sudo systemctl restart lqosd lqos_scheduler
```

### Hotfix de Ubuntu 24.04 si la actualización se detiene

En hosts Ubuntu 24.04 afectados que usan `systemd-networkd`, `apt install` puede detenerse y mostrar un mensaje requiriendo el hotfix. Esto es esperado.

Si eso ocurre, ejecuta:

```bash
sudo /opt/libreqos/src/systemd_hotfix.sh install
sudo reboot
```

El instalador del hotfix configura el repositorio APT firmado de LibreQoS en `https://repo.libreqos.com`, instala el conjunto parchado de paquetes `systemd` de Noble y fija esos paquetes para futuras actualizaciones.
De forma predeterminada, selecciona la versión actual del hotfix desde ese repositorio.
Si Soporte de LibreQoS te indica que tu paquete instalado es anterior a v2.2, usa este comando de recuperación en su lugar:

```bash
sudo HOTFIX_PACKAGE_VERSION=255.4-1ubuntu9999+libreqos1 /opt/libreqos/src/systemd_hotfix.sh install
sudo dpkg --configure -a
sudo reboot
```

Después del reinicio, reanuda la actualización y reinicia los servicios:

```bash
cd /tmp
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
sudo systemctl restart lqosd lqos_scheduler
```

### Reinicio posterior a la actualización

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

### Hotfix de Ubuntu 24.04 para instalaciones desde Git

Antes de reiniciar los servicios de LibreQoS en Ubuntu 24.04, verifica si se debe ofrecer el hotfix de Noble para `systemd-networkd`:

```bash
cd /opt/libreqos/src
./systemd_hotfix.sh status
```

Si el script indica que el hotfix debe ofrecerse, instálalo y reinicia antes de continuar:

```bash
sudo ./systemd_hotfix.sh install
sudo reboot
```

El instalador selecciona la versión actual del hotfix desde el repositorio APT firmado de LibreQoS. Soporte de LibreQoS puede proporcionar un valor `HOTFIX_PACKAGE_VERSION` para un incidente específico.

Ejecuta los siguientes comandos para recargar los servicios de LibreQoS:

```shell
sudo systemctl restart lqosd lqos_scheduler
```

## Síntomas para pausar y hacer triage

Detenga el rollout y haga triage si aparece:

- `lqosd` o `lqos_scheduler` no saludable tras reinicio
- cambio inesperado de jerarquía tras sync de integración
- scheduler persistentemente no saludable en WebUI
