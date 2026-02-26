# Quickstart: Elija su ruta de despliegue

LibreQoS soporta múltiples rutas reales de despliegue. Comience eligiendo una ruta y mantenga un único modelo de fuente de verdad para los datos de shaping.

## Antes de empezar

- La WebUI es la interfaz operativa principal después de instalar.
- Elija un modelo de fuente de verdad para `network.json` y `ShapedDevices.csv`.
- Evite ediciones en competencia entre múltiples sistemas para los mismos objetos.

## Testbed / Lab

Use esta ruta cuando quiera validar el comportamiento de LibreQoS en un entorno controlado antes de desplegar inline en producción.

### Flujo de configuración de laboratorio

1. Arme una topología de laboratorio (caja LibreQoS + endpoints generadores de tráfico + routers opcionales para simular rutas OSPF/BGP).
2. Complete plataforma y prerrequisitos de instalación:
   - [Escenarios de Despliegue](design-es.md)
   - [Requisitos del Sistema](requirements-es.md)
   - [Configuración del Servidor - Prerrequisitos](prereq-es.md)
   - [Instalar Ubuntu Server 24.04](ubuntu-server-es.md)
   - [Configurar Puente de Regulación](bridge-es.md)
3. Instale LibreQoS con paquete `.deb` (recomendado):

```bash
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

4. Elija una pista de prueba:
   - **Lab con archivos manuales**: use `network.json` y `ShapedDevices.csv` manuales/de ejemplo.
   - **Lab con integración soportada**: configure integración y valide datos importados.
5. Genere tráfico de prueba y valide en WebUI:
   - comportamiento de Dashboard/Tree/Flow
   - estado del scheduler
   - problemas urgentes
6. Cuando el comportamiento sea el esperado, avance a una ruta de piloto inline.

### Después de esta ruta

- [Configurar LibreQoS](configuration-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Integración Soportada

Use esta ruta cuando su CRM/NMS esté soportado por las integraciones incluidas de LibreQoS.

1. Complete prerrequisitos e instale LibreQoS (mismos enlaces/pasos anteriores).
2. Configure la integración en WebUI.
3. Valide en WebUI los datos importados de shaping y topología.
4. Coloque la caja LibreQoS inline para tráfico piloto.
5. Monitoree en WebUI la salud del scheduler, comportamiento de shaping y problemas urgentes.
6. Expanda del piloto a un despliegue más amplio tras observar estabilidad.

Nota:
- En modo integración, `ShapedDevices.csv` normalmente se regenera por los jobs de sincronización.
- El comportamiento de sobrescritura de `network.json` depende de la configuración (por ejemplo `always_overwrite_network_json`).

### Después de esta ruta

- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Integración con Script Personalizado

Use esta ruta cuando su CRM/NMS no esté soportado y usted generará `network.json` y `ShapedDevices.csv` con scripts propios.

1. Complete prerrequisitos e instale LibreQoS (mismos enlaces/pasos anteriores).
2. Implemente script/proceso para generar y refrescar `network.json` y `ShapedDevices.csv`.
3. Trate las salidas del script como fuente de verdad para persistencia de largo plazo.
4. Coloque la caja LibreQoS inline para tráfico piloto.
5. Use WebUI para verificaciones operativas y cambios rápidos.
6. Lleve cambios lógicos/de datos permanentes de vuelta a su pipeline de scripts.

Nota:
- Las ediciones por WebUI son útiles para cambios operativos rápidos.
- El estado de largo plazo debe mantenerse en su flujo externo de fuente de verdad.

### Después de esta ruta

- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Archivos Manuales (<100 suscriptores)

Use esta ruta cuando mantenga intencionalmente `network.json` y `ShapedDevices.csv` de forma directa, sin sincronización CRM/NMS.

Recomendado solo para redes menores a 100 suscriptores.

1. Complete prerrequisitos e instale LibreQoS (mismos enlaces/pasos anteriores).
2. Construya y mantenga `network.json` y `ShapedDevices.csv` directamente.
3. Coloque la caja LibreQoS inline para tráfico piloto.
4. Use WebUI para validar shaping, estado del scheduler y comportamiento de topología.
5. Mantenga disciplina estricta en actualizaciones manuales (calidad de datos, consistencia y cadencia).
6. Si crece el número de suscriptores o el volumen de cambios, planifique migrar a integración soportada o integración con scripts.

### Después de esta ruta

- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Escalado y diseño de topología](scale-topology-es.md)
- [Solución de problemas](troubleshooting-es.md)

## Instalación para desarrolladores (No recomendada para operadores)

La instalación basada en Git sigue disponible aquí:
- [Git Install (Para Desarrolladores)](git-install-es.md)
