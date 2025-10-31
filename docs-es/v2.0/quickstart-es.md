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
wget {deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

### Instalación con Git (Solo para Desarrolladores - No Recomendado)

[Instalación Compleja](git-install-es.md)

## Paso 4 - Configurar LibreQoS

¡Ahora estás listo para [Configurar](configuration-es.md) LibreQoS!
