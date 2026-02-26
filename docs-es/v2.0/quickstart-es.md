# Quickstart: Elija su ruta de despliegue

Use esta página para elegir rápidamente la ruta correcta y ejecutar solo los pasos de esa ruta.

## Base de instalación común

Complete esto una vez antes de cualquier paso específico de ruta.

1. Revise supuestos de despliegue y capacidad:
   - [Escenarios de Despliegue](design-es.md)
   - [Requisitos del Sistema](requirements-es.md)
2. Prepare host y sistema operativo:
   - [Configuración del Servidor - Prerrequisitos](prereq-es.md)
   - [Instalar Ubuntu Server 24.04](ubuntu-server-es.md)
3. Configure el modo de puente:
   - [Configurar Puente de Regulación](bridge-es.md)
4. Instale LibreQoS (`.deb` recomendado):

```bash
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

5. Abra WebUI en `http://your_shaper_ip:9123`.

## Después de instalar: estado esperado (10 minutos)

Antes de cambios profundos, valide base operativa:

1. Servicios activos:
```bash
sudo systemctl status lqosd lqos_scheduler
```
2. WebUI carga y actualiza (Dashboard + Scheduler Status).
3. Sin errores críticos recientes:
```bash
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```
4. Fuente de verdad definida:
- modo integración: la integración controla refresco de archivos
- modo custom/manual: sus archivos/scripts controlan persistencia

Si falla algo, pase a [Solución de problemas](troubleshooting-es.md) antes del piloto.

## Testbed / Lab

### Cuándo elegir

Quiere validar comportamiento en un entorno controlado antes de producción inline.

### Haga esto ahora

1. Arme topología de laboratorio (LibreQoS + endpoints generadores de tráfico + simulación opcional OSPF/BGP).
2. Elija una pista de laboratorio:
   - **Lab con archivos manuales**: validar comportamiento de `network.json` + `ShapedDevices.csv`.
   - **Lab con integración soportada**: validar datos importados de suscriptores/topología.
3. Genere tráfico y valide en WebUI (Dashboard, Tree, Flow, Scheduler Status, Urgent Issues).
4. Confirme comportamiento de shaping y estabilidad esperada.

### Luego vaya aquí

- [Configurar LibreQoS](configuration-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Integración Soportada

### Cuándo elegir

Su CRM/NMS está soportado por integraciones incluidas de LibreQoS.

### Haga esto ahora

1. Configure la integración en WebUI.
2. Ejecute sincronización inicial y valide datos importados.
3. Coloque LibreQoS inline para tráfico piloto.
4. Valide señales de salud en WebUI (Scheduler Status, Urgent Issues, vistas de topología/flujo).
5. Expanda alcance del piloto tras operación estable.

Nota:
- En modo integración, `ShapedDevices.csv` normalmente se regenera por los jobs de sincronización.
- La sobrescritura de `network.json` depende de configuración (por ejemplo `always_overwrite_network_json`).

### Luego vaya aquí

- [Integraciones CRM/NMS](integrations-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Errores comunes en primera puesta en marcha

- Fuente de verdad no definida entre integración y edición manual.
- Elegir estrategia topológica profunda sin validar salud base.
- Omitir checks de servicios/logs antes de tráfico piloto.

## Integración con Script Personalizado

### Cuándo elegir

Su CRM/NMS no está soportado y usted generará `network.json` + `ShapedDevices.csv` con su propio pipeline.

### Haga esto ahora

1. Implemente script/proceso para generar y refrescar archivos de shaping.
2. Declare salidas del script como fuente de verdad.
3. Coloque LibreQoS inline para tráfico piloto.
4. Use WebUI para verificaciones operativas y ajustes rápidos.
5. Lleve cambios permanentes de vuelta al flujo de scripts.

Nota:
- Las ediciones por WebUI son útiles para operación rápida.
- El estado de largo plazo debe mantenerse en su flujo externo de fuente de verdad.

### Luego vaya aquí

- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Archivos Manuales (<100 suscriptores)

### Cuándo elegir

Mantiene intencionalmente `network.json` + `ShapedDevices.csv` sin sincronización CRM/NMS.

Recomendado solo para redes menores a 100 suscriptores.

### Haga esto ahora

1. Construya y mantenga archivos de shaping directamente.
2. Coloque LibreQoS inline para tráfico piloto.
3. Valide shaping y estado del scheduler en WebUI.
4. Mantenga disciplina estricta de cambios manuales.
5. Planifique migración a integración soportada o scripts si crece escala/volumen de cambios.

### Luego vaya aquí

- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)
