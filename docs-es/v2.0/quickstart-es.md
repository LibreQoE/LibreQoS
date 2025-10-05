# Instalar LibreQoS

## Paso 1 - Validar Suposiciones de Diseño de Red y Selección de Hardware

- [Suposiciones de Diseño de Red](design-es.md)
- [Requisitos del Sistema](requirements-es.md)

## Paso 2 - Completar los Prerrequisitos de Instalación

- [Configuración del Servidor - Prerrequisitos](prereq-es.md)
- [Instalar Ubuntu Server 24.04](ubuntu-server-es.md)
- [Configurar Puente de Regulación](bridge-es.md)

## Paso 3 - Instalar LibreQoS v1.5 / Actualizar a LibreQoS v1.5

### Usar Paquete .DEB (Método Recomendado)

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget http://download.libreqos.com/libreqos_1.5-RC2.202510052233-1_amd64.deb
sudo apt install ./libreqos_1.5-RC2.202510052233-1_amd64.deb
```

### Instalación con Git (Solo para Desarrolladores - No Recomendado)

[Instalación Compleja](git-install-es.md)

## Paso 4 - Configurar LibreQoS

¡Ahora estás listo para [Configurar](configuration-es.md) LibreQoS!
