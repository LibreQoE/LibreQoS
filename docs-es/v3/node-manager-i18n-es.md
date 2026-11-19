# Idioma de la interfaz web

El marco del dashboard de `lqosd` se puede mostrar en inglés o en español. Esta
página cubre dónde se elige, cómo se recuerda, el valor predeterminado del nodo,
qué alcanza realmente la traducción y cómo agregar otro idioma.

Aplica al dashboard local en `http://ip_del_shaper:9123`. El resto del flujo de
operación se documenta en el conjunto de documentación de LibreQoS v3 (en inglés).

## Elegir un idioma

- Elija el idioma en el selector de idioma de la barra lateral, en su propio
  elemento justo encima de `Display` (`Apariencia`). Cada opción aparece en su
  propio idioma (`English`, `Español`) y la activa queda preseleccionada. Antes
  de iniciar sesión, el mismo selector aparece en la esquina superior derecha de
  las pantallas de inicio de sesión y de primera ejecución, así que una
  instalación nueva puede cambiarse a español en la primera página que muestra.
- La elección pertenece al navegador, no a la cuenta de usuario: se guarda en una
  cookie `lqos_lang` por un año, así que se mantiene al recargar y al reiniciar el
  navegador en esa máquina, y no cambia lo que ve otro operador.
- Para volver al valor del nodo, elimine la cookie `lqos_lang` de ese nodo y
  recargue: la página sigue `ui_language` y, cuando el nodo no lo define, el
  idioma de su navegador.
- Orden de resolución de una página: la elección del navegador (`lqos_lang`),
  luego `ui_language`, luego el encabezado `Accept-Language` del navegador y, por
  último, inglés. Si `ui_language` tiene un tag para el que esta compilación no
  trae catálogo, se ignora (el log deja un aviso) en lugar de romper la página.
- El idioma servido se informa en el encabezado `Content-Language` y en el
  atributo `lang` de la página, así que los lectores de pantalla, el corrector
  ortográfico y la traducción del navegador coinciden con lo que se ve en
  pantalla.
- Las páginas se sirven con `Cache-Control: no-store`, de modo que un navegador
  compartido detrás de un proxy no reciba el idioma de otro operador.

## Valor predeterminado del nodo

Defina la clave `ui_language` en el nivel superior de `/etc/libreqos/lqos.conf` y
reinicie `lqosd`. Todo navegador sin elección propia comienza en ese idioma; una
elección explícita en el navegador sigue teniendo prioridad.

```toml
ui_language = "es"
```

## Qué está traducido y qué no

- Traducido: el marco. Es decir, la barra lateral y sus etiquetas de sección, el
  menú `Display`, el selector de idioma, el buscador, el panel de alertas
  (conteos, estado vacío, acciones para reconocer alertas), los tiempos
  relativos como `hace 5 min`, y las páginas independientes de inicio de sesión
  y primer inicio.
- No traducido: el cuerpo de las páginas, las tablas, las etiquetas de los
  gráficos, el contenido de ayuda, los textos largos del scheduler y del bridge, y
  los mensajes de alerta, que provienen del servidor.
- Por lo tanto, una selección en español da un marco en español con contenido en
  inglés. Considérelo un ajuste de idioma de la interfaz, no un producto
  traducido.

## Agregar un idioma

Los catálogos son datos, así que agregar un idioma no requiere cambios en Rust ni
en JavaScript. El inglés es el único idioma que el código todavía nombra: debe
seguir listado en `locales.json` y es el respaldo integrado, tanto en el servidor
como en el tiempo de ejecución del navegador. Las rutas siguientes son relativas
al árbol de fuentes de LibreQoS (`src/` en un clon). Agregar un idioma es un
cambio de compilación, así que un nodo instalado lo recibe al instalar un paquete
reconstruido.

1. Agregue el catálogo y lístelo en la misma edición. El compilador rechaza cada
   mitad por separado: falla tanto un catálogo que ninguna entrada lista como un
   tag listado sin catálogo.
   - Copie `rust/lqosd/src/node_manager/static2/i18n/en.json` a `<tag>.json` en
     ese directorio (por ejemplo `de.json`) y traduzca los valores de `messages`.
     Mantenga las claves y los nombres de `{placeholder}` sin cambios.
   - En el archivo copiado, defina el `"locale"` de nivel superior con el nuevo
     tag y `"dir"` con la dirección que llevará su entrada en la lista (`ltr` o
     `rtl`); el compilador falla si alguno de los dos campos no coincide con esa
     entrada.
   - Agregue la entrada en
     `rust/lqosd/src/node_manager/static2/i18n/locales.json` con el mismo `tag`,
     el `name` nativo que se muestra en el selector y ese mismo `dir`. La entrada
     aparece solo con esta lista.
2. Copie los recursos al lugar donde `lqosd` los sirve, luego recompile y reinicie
   el servicio. Ejecute `rust/lqosd/copy_files.sh`: ejecuta `esbuild.sh` desde
   `rust/lqosd/src/node_manager/js_build` (que regenera `i18n/*.js` a partir del
   JSON), ejecuta el contrato de compilación y copia el resultado a `bin/static2`
   en el árbol de fuentes, que es el directorio que el daemon sirve como
   `<lqos_directory>/bin/static2`. `esbuild.sh` por sí solo únicamente llena
   `js_build/out/`, que el daemon nunca lee, así que una página servida como
   `<tag>` volvería al inglés sin avisar. Después recompile y reinicie `lqosd`,
   que compila la lista de idiomas dentro del binario. En un sistema instalado,
   reconstruya e instale el `.deb` desde un clon con `build_dpkg.sh` (ejecuta
   `copy_files.sh` por usted y empaqueta `/opt/libreqos/src/bin/static2`).

Una clave que falte conserva su texto en inglés, así que una traducción parcial se
puede publicar sin riesgo.
