# Instalar LibreQoS 1.4

## Actualización desde la versión v1.3

### Eliminar offloadOff.service

```shell
sudo systemctl disable offloadOff.service
sudo rm /usr/local/sbin/offloadOff.sh /etc/systemd/system/offloadOff.service
```

### Eliminar las tareas cron de la versión v1.3

Ejecute ```sudo crontab -e``` y elimine cualquier entrada relacionada con LibreQoS desde v1.3.

## Instalación sencilla desde el paquete .Deb (recomendada)

Utilice el paquete .deb de la [última versión v1.4 ](https://github.com/LibreQoE/LibreQoS/releases/).

```shell
sudo echo "deb http://stats.libreqos.io/ubuntu jammy main" | sudo tee -a /etc/apt/sources.list.d/libreqos.list
sudo wget -O - -q http://stats.libreqos.io/repo.asc | sudo apt-key add -
apt-get update
apt-get install libreqos
```

Se le realizarán algunas preguntas sobre su configuración, y el demonio de administración y el servidor web se iniciarán automáticamente. Vaya a http://<your_ip>:9123/ para finalizar la instalación.

## Instalación compleja (no recomendada)

```{note}
Use esta instalación si quiere implementar constantemente desde la rama principal en Github. ¡Solo para usuarios experimentados!
```

[Instalación Compleja](../TechnicalDocs/complex-install.md)

Ahora está listo para [Configurar](./configuration.md) LibreQoS!
