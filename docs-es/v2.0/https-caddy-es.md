# HTTPS opcional con Caddy

LibreQoS puede publicar la WebUI y la documentación de la API detrás de Caddy para que los operadores usen HTTPS. Esto es opcional. LibreQoS sigue funcionando sin esta función.

## Cuándo usarlo

Use `Configuration -> SSL Setup` cuando quiera que los operadores abran LibreQoS con HTTPS en lugar de HTTP plano en el puerto `9123`.

También puede habilitar la misma opción durante el flujo de configuración inicial.

## Dos modos de certificado

LibreQoS ofrece dos opciones simples:

- Ingrese un hostname externo como `libreqos.example.com`: Caddy solicitará un certificado público a Let's Encrypt. Los navegadores deberían confiar en él automáticamente una vez que DNS y el acceso entrante estén correctos.
- Deje el hostname vacío: Caddy protegerá LibreQoS por dirección IP de gestión usando la autoridad certificadora local de Caddy. El tráfico sigue cifrado, pero las computadoras de los operadores deben confiar en el certificado raíz local de Caddy antes de que desaparezcan las advertencias del navegador.

## Qué cambia después de habilitarlo

- Los operadores dejan de usar `http://tu_ip_del_shaper:9123` y pasan a usar `https://tu-hostname/` o `https://tu-ip-de-gestión/`.
- LibreQoS mueve el listener de la WebUI a `127.0.0.1:9123`.
- Caddy publica la WebUI y la documentación de la API por HTTPS.
- Swagger pasa a `/api/v1/api-docs` en el mismo origen HTTPS de la WebUI.

## Si usa el modo de certificado local

Cuando deja el hostname vacío, el certificado raíz local de Caddy se guarda en el host LibreQoS en:

```text
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
```

Confíe en ese certificado en cada estación de trabajo de operador que vaya a abrir la WebUI por HTTPS.

## Deshabilitar HTTPS

Si quiere revertir el cambio, abra `Configuration -> SSL Setup` y elija `Disable SSL`.

LibreQoS entonces:

- elimina la configuración gestionada de Caddy
- restaura el listener directo anterior de la WebUI, o el valor predeterminado normal `:::9123` si antes no había un listener personalizado
- devuelve a los operadores el acceso HTTP directo por la IP de gestión y el puerto `9123`

## Páginas relacionadas

- [Inicio rápido](quickstart-es.md)
- [Configurar LibreQoS](configuration-es.md)
- [Interfaz WebUI (Node Manager)](node-manager-ui-es.md)
- [API del nodo de LibreQoS](api-es.md)
- [Solución de problemas](troubleshooting-es.md)
