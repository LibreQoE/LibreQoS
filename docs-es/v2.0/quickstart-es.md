# Quickstart: Ruta de Despliegue para ISP

Use esta página para pasar de instalación a piloto seguro con mínima ambigüedad.

¿Necesita definiciones de términos clave? Vea el [Glosario](glossary-es.md).

## 1) Base de instalación común

Complete esto una vez:

1. Revise arquitectura y dimensionamiento:
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

## 2) Puerta de salud en 10 minutos (obligatoria antes del piloto)

Ejecute:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```

Confirme:
- Carga WebUI Dashboard.
- Scheduler Status está saludable.
- No hay errores urgentes/fatales de arranque en logs.

Si falla esto, vaya a [Solución de problemas](troubleshooting-es.md) antes de continuar.

## 3) Decisión A: Etapa de despliegue

Elija una:

- **Laboratorio primero**: validar comportamiento en entorno controlado antes de tráfico inline.
- **Piloto inline ahora**: avanzar directo con alcance limitado de tráfico productivo.

Si elige laboratorio primero:
1. Arme topología de laboratorio.
2. Genere tráfico de prueba.
3. Valide Dashboard, Tree, Flow, Scheduler Status y Urgent Issues.
4. Continúe a la Decisión B.

## 4) Decisión B: Fuente de verdad (elija un dueño)

| Si esto describe su caso | Modo | Dueño de datos de shaping durables |
|---|---|---|
| Usa integración CRM/NMS soportada | Modo integración incluida | Jobs de integración |
| Genera `network.json` y `ShapedDevices.csv` con scripts propios | Modo fuente de verdad personalizada | Sus scripts |
| Mantiene archivos manualmente en red pequeña/simple | Modo archivos manuales | Edición manual |

Regla: mantenga un único dueño para insumos persistentes de shaping.

## 5) Tarjetas de ruta

### Modo Integración Incluida

Cuándo elegir:
- Su CRM/NMS está soportado por integraciones incluidas.

Haga esto ahora:
1. Configure integración en WebUI.
2. Ejecute sincronización inicial y valide datos importados de shaping/topología.
3. Coloque LibreQoS inline para tráfico piloto.
4. Valide Scheduler Status, Urgent Issues y vistas de topología/flujo.
5. Expanda alcance del piloto tras estabilidad.

Siguiente:
- [Integraciones CRM/NMS](integrations-es.md)
- [Solución de problemas](troubleshooting-es.md)

### Modo Fuente de Verdad Personalizada (Sus Scripts)

Cuándo elegir:
- Su CRM/NMS no está soportado y usted generará `network.json` + `ShapedDevices.csv` con su pipeline propio.

Haga esto ahora:
1. Implemente script/proceso para generar y refrescar archivos de shaping.
2. Declare salidas del script como su fuente de verdad.
3. Coloque LibreQoS inline para tráfico piloto.
4. Use WebUI para checks operativos y ajustes de corto plazo.
5. Mantenga cambios permanentes en su flujo externo de scripts.

Referencia de formato:
- Vea las secciones `network.json` y `ShapedDevices.csv` en [Referencia avanzada de configuración](configuration-advanced-es.md).

Siguiente:
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Solución de problemas](troubleshooting-es.md)

### Modo Archivos Manuales (<100 suscriptores)

Cuándo elegir:
- Mantiene intencionalmente `network.json` + `ShapedDevices.csv` sin sincronización CRM/NMS.

Haga esto ahora:
1. Construya y mantenga archivos de shaping directamente.
2. Coloque LibreQoS inline para tráfico piloto.
3. Valide shaping y estado del scheduler en WebUI.
4. Mantenga disciplina estricta de cambios manuales.
5. Planifique migración a integración soportada o scripts si crece escala/volumen de cambios.

Siguiente:
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)

## 6) Errores comunes en primera puesta en marcha

- Propiedad poco clara de la fuente de verdad entre integración y edición manual.
- Cambiar profundidad topológica antes de pasar la puerta de salud.
- Omitir validación de servicios/logs antes de tráfico piloto.

## 7) Páginas relacionadas

- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)
