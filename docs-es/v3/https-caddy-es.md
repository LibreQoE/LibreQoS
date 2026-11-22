# HTTPS opcional con Caddy

**Versión: LibreQoS v3.**

LibreQoS puede poner la WebUI y la API detrás de Caddy para que los operadores
accedan por HTTPS. Es opcional; LibreQoS funciona sin ello.

## Activarlo

Abra **Configuration → SSL Setup** en la WebUI y elija **Setup SSL**. La
configuración inicial ofrece la misma opción. LibreQoS instala Caddy y escribe
su configuración por usted.

## Dos modos de certificado

- **Indique un nombre de host externo**, como `libreqos.example.com`. Caddy
  solicita un certificado público a Let's Encrypt. Los navegadores confían en él
  automáticamente cuando el DNS y el acceso entrante al host son correctos.
- **Deje el nombre de host vacío.** Caddy protege LibreQoS por su dirección IP
  de administración, con la autoridad de certificación local de Caddy. El
  tráfico sigue cifrado, pero cada equipo de operador debe confiar en el
  certificado raíz local de Caddy para que desaparezcan las advertencias del
  navegador.

## Qué cambia

- Los operadores usan `https://su-nombre-de-host/` o
  `https://su-ip-de-administracion/` en lugar de `http://ip_del_shaper:9123`.
- El listener de la WebUI pasa a `127.0.0.1:9123` y Caddy hace de proxy hacia él.
- La API en `/api/v3`, con su documentación en `/api/v3/docs`, forma parte de
  `lqosd` y se sirve en la misma dirección HTTPS. No hay un segundo upstream que
  configurar. Cada solicitud a la API sigue necesitando una clave de API local en
  la cabecera `x-bearer`; cree las claves en
  **Configuration → License & Services → Local API Keys**.

HTTPS solo cubre la interfaz de administración. Los puertos del dataplane DPDK
no se ven afectados.

## Modo de certificado local

Con el nombre de host vacío, el certificado raíz local de Caddy está en:

```text
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
```

Confíe en ese certificado en cada estación de operador que abra la WebUI.

## Desactivarlo

Abra **Configuration → SSL Setup** y elija **Disable SSL**. LibreQoS entonces:

- elimina la configuración de Caddy que administra;
- restaura el listener de la WebUI que había antes de activar HTTPS o, si no
  había ninguno, el predeterminado (puerto `9123` en todas las direcciones IPv4);
- devuelve a los operadores a HTTP sin cifrar en la IP de administración y el
  puerto `9123`.

En hosts que activaron HTTPS con una beta anterior a beta20.23, desactivarlo puede dejar la
WebUI en `127.0.0.1:9123`, que solo es accesible desde el propio host. Para
corregirlo, elimine la línea `webserver_listen` de `/etc/libreqos/lqos.conf` (o
asígnele `"0.0.0.0:9123"`) y ejecute `sudo systemctl restart libreqos-lqosd`.
