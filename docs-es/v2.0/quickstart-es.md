# Inicio rápido: ruta de despliegue por WebUI

Use esta página para pasar de la instalación del paquete a un piloto seguro con la menor cantidad posible de ambigüedad.

Siga esta página en orden:
1. Complete la base común de instalación.
2. Abra la WebUI y cree el primer usuario administrador si hace falta.
3. Use `Complete Setup` para elegir de dónde vendrán la topología y los suscriptores.
4. Pase la verificación de salud de 10 minutos.
5. Comience con tráfico piloto limitado antes de ampliar el despliegue.

¿Necesita definiciones de términos clave? Vea el [Glosario](glossary-es.md).

## 1) Base común de instalación

Complete esto una sola vez antes de intentar regular tráfico en producción:

1. Revise arquitectura y dimensionamiento:
- [Escenarios de Despliegue](design-es.md)
- [Requisitos del Sistema](requirements-es.md)

2. Prepare el host y el sistema operativo:
- [Configuración del Servidor - Prerrequisitos](prereq-es.md)
- [Instalar Ubuntu Server 24.04](ubuntu-server-es.md)

3. Configure cómo pasará el tráfico por el equipo LibreQoS:
- [Configurar Puente de Regulación](bridge-es.md)

4. Instale LibreQoS (`.deb` recomendado):

```bash
cd /tmp
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

Usar `/tmp` evita problemas de permisos con `.deb` locales cuando `apt` no puede leer un paquete guardado en un directorio home privado con el usuario `_apt`.

### Hotfix de Ubuntu 24.04 si la instalación del `.deb` se detiene

En hosts Ubuntu 24.04 afectados que usan `systemd-networkd`, la instalación del `.deb` puede detenerse y mostrar un mensaje requiriendo el hotfix. Esto es esperado.

Si eso ocurre, ejecute:

```bash
sudo /opt/libreqos/src/systemd_hotfix.sh install
sudo reboot
```

El instalador del hotfix configura el repositorio APT de LibreQoS en `https://repo.libreqos.com`, instala el conjunto parchado de paquetes `systemd` de Noble y fija esos paquetes para futuras actualizaciones.

Después del reinicio, reanude la instalación:

```bash
cd /tmp
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

## 2) Abra la WebUI y complete el primer inicio de sesión

1. Abra la WebUI en `http://your_shaper_ip:9123`.
2. Si todavía no existen usuarios de WebUI, LibreQoS redirige a `first-run.html`.
3. Cree el usuario administrador inicial si se lo pide.
4. Inicie sesión y confirme que carga el Dashboard.

Opcional: si los operadores necesitan HTTPS, habilite `Configuration -> SSL Setup` después de iniciar sesión, o active la misma opción durante la configuración inicial. Vea [HTTPS opcional con Caddy](https-caddy-es.md).

En este punto, que la WebUI responda no demuestra todavía que LibreQoS ya esté listo para hacer shaping de suscriptores.

Si Scheduler Status muestra `Setup Required`, eso es normal hasta que elija una fuente de topología y publique datos válidos de shaping.

## 3) Use `Complete Setup` para elegir la fuente de topología

Después de poder iniciar sesión, abra `Complete Setup`.

Esta página es donde la mayoría de los ISP deben tomar la siguiente decisión. Elija una sola fuente de verdad para los cambios permanentes de shaping:

| Si esto describe su caso | Use esta ruta | Dónde deben hacerse los cambios permanentes |
|---|---|---|
| Usa un CRM/NMS soportado como UISP, Splynx, VISP, Netzur, Powercode, Sonar o WispGate | Integración incluida | Su sistema de integración y su configuración de LibreQoS |
| Ya tiene su propio importador interno | Importador personalizado | Su script o proceso externo |
| Quiere mantener los archivos manualmente de forma intencional | Archivos manuales | `network.json` y `ShapedDevices.csv` |

Regla: elija un solo lugar para los cambios permanentes. No mezcle ediciones manuales con refrescos programados de integración salvo que realmente quiera esa complejidad.

### Integración incluida

Esta es la ruta recomendada para la mayoría de los ISP.

Haga esto ahora:
1. Abra desde `Complete Setup` la página de su proveedor.
2. Guarde la configuración de la integración.
3. Ejecute la sincronización inicial o espere la primera importación programada.
4. Vuelva a Scheduler Status y confirme que LibreQoS ya no está esperando la configuración inicial.

Siguiente:
- [Integraciones CRM/NMS](integrations-es.md)
- [Solución de problemas](troubleshooting-es.md)

### Importador personalizado

Elija esta opción solo si otro proceso interno ya escribe archivos compatibles con LibreQoS.

Haga esto ahora:
1. Configure el comportamiento compartido de topología en `Integration - Common`.
2. Publique `network.json` y `ShapedDevices.csv` desde su propio proceso.
3. Recargue o espere al scheduler para que LibreQoS valide y use esos archivos.

Siguiente:
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)

### Archivos manuales

Elija esta opción solo si quiere que LibreQoS mantenga directamente esos archivos.

Haga esto ahora:
1. Construya `network.json`.
2. Construya `ShapedDevices.csv`.
3. Manténgalos con los editores de WebUI o con su flujo basado en archivos.
4. Confirme que el scheduler acepta los datos y que aparece la topología esperada.

Siguiente:
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)

## 4) Verificación de salud de 10 minutos

Después de terminar `Complete Setup` y de que su fuente elegida haya publicado datos válidos, ejecute:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```

Confirme:
- El Dashboard carga.
- `lqosd` y `lqos_scheduler` están activos.
- Scheduler Status ya no muestra `Setup Required`.
- Scheduler Status está saludable, o muestra trabajo activo esperado sin errores de validación ni de arranque.
- No aparecen problemas urgentes o fatales de inicio en los logs.
- La topología o la lista esperada de suscriptores/dispositivos aparece en WebUI.

Si esto falla, vaya a [Solución de problemas](troubleshooting-es.md) antes de pasar tráfico piloto.

## 5) Comience con un piloto limitado

No empiece con un despliegue inline amplio.

Empiece con un piloto pequeño y confirme:
- un suscriptor o dispositivo de prueba hace shaping como se espera
- aparecen los nodos padre y la profundidad de jerarquía esperados
- Scheduler Status se mantiene saludable después de los refrescos
- no aparecen nuevos errores urgentes en logs después de los primeros ciclos

Amplíe solo después de tener una base conocida y estable.

## 6) Errores comunes al inicio

- Suponer que `Dashboard loads` significa que el shaping ya está listo.
- Ignorar `Setup Required` y asumir que el scheduler ya está regulando clientes.
- Mezclar datos controlados por integración con ediciones manuales de archivos.
- Cambiar demasiados detalles de topología antes de una verificación limpia.
- Empezar un despliegue amplio antes de validar un piloto pequeño.

## 7) El día 1 termina cuando

- Puede iniciar sesión correctamente.
- El Dashboard carga.
- `Complete Setup` quedó terminado para el flujo elegido.
- Scheduler Status ya no muestra `Setup Required`.
- No quedan problemas urgentes o fatales de arranque.
- Aparece la topología o la lista de suscriptores esperada.
- Un suscriptor o dispositivo piloto se comporta como se espera.

## 8) Páginas relacionadas

- [Configurar LibreQoS](configuration-es.md)
- [HTTPS opcional con Caddy](https-caddy-es.md)
- [Modos de operación y fuente de verdad](operating-modes-es.md)
- [Integraciones CRM/NMS](integrations-es.md)
- [Referencia avanzada de configuración](configuration-advanced-es.md)
- [Solución de problemas](troubleshooting-es.md)
