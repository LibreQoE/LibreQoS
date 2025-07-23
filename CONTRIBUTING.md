> Este es el **borrador 1** de la guía de contribuciones. Se agradecen comentarios y modificaciones.

# Contribuyendo a LibreQoS

¿Te interesa contribuir a LibreQoS? ¡Genial! No dudes en colaborar con lo que te interese. Estaremos encantados de ayudarte.

Consulta nuestro [Código de Conducta](https://github.com/LibreQoE/LibreQoS/blob/main/.github/CODE_OF_CONDUCT.md). Queremos que este sea un espacio agradable y constructivo. Además, únete al chat en [Matrix](https://app.element.io/#/room/#libreqos:matrix.org). Los desarrolladores principales están ahí y estarán encantados de ayudarte.

En particular:

* Podemos comprobar que no se esté trabajando en un problema. ¡No hay nada más frustrante que trabajar duro en un problema y descubrir que alguien ya lo ha solucionado!
* Podemos ayudarle a señalar temas que podrían interesarle.
* Podemos ofrecer ayuda y/o tutoría con Rust y C.

# Cómo puedes ayudar

Hay muchas maneras en las que puedes ayudar:

* **Prueba de batalla de LibreQos**: usa el software y cuéntanos qué funciona y qué no.
* **Donar**: es software gratuito, pero agradecemos tu apoyo.
* **Háganos saber lo que piensa**: ingrese al chat o a nuestra página de discusiones y cuéntenos lo que piensa.
* **Encontrando errores**: ¿Algo no te funciona? No podemos solucionarlo si no nos lo dices.
* **Corrección de errores** y **Adición de funciones**: consulte la "Guía de desarrollo" a continuación.
* **Enseñando a otros**: Si LibreQoS te funciona bien, puedes ayudar a otros compartiendo tu experiencia. ¿Ves a alguien con dificultades? No dudes en colaborar.

LibreQos se esfuerza por crear un ambiente abierto y amigable. Si tienes alguna idea para ayudar, háznosla saber.

# Directrices de desarrollo

Esta sección contiene algunos consejos para ayudarle a comenzar a escribir código para LibreQoS.

## Orientación con LibreQoS

LibreQos se divide en varias secciones:

* **Rust: plano de control/gestión del sistema**
   * **Demonios del sistema**
      *`lqosd` es probablemente la parte más importante del sistema Rust. Carga el sistema eBPF, que proporciona control de puenteo y modelado de tráfico, gestiona el bus para la comunicación con otros sistemas y recopila información directamente del eBPF y del kernel. Cuenta con varias subcajas anidadas:
         *`lqos_heimdall` que maneja todo el rastreo de paquetes, el seguimiento de flujo y los datos de paquetes compatibles con `libcap`.
         * `lqos_queue_tracker` mantiene estadísticas para las colas de modelado `tc` de Linux, particularmente Cake.
      * `lqos_node_manager` proporciona una interfaz web de gestión por nodo. Recopila la mayoría de sus datos desde `lqosd` a través del bus.
   * **Utilidades CLI**
      * `lqusers` proporciona una interfaz CLI para administrar la autenticación en el administrador de nodos.
      * `lqtop` proporciona una manera rápida y sencilla de ver lo que está haciendo el modelador desde la consola de administración local.
      * `xdp_iphash_to_cpu_cmdline` (el nombre se hereda de proyectos anteriores) proporciona una interfaz de línea de comandos para asignar subredes IP a controladores de TC y CPU.
      * `xdp_pping`Proporciona una herramienta CLI para brindar un resumen rápido de los tiempos RTT de TCP, agrupados por identificador de TC.
   * **Bibliotecas**
      * `lqos_bus` Proporciona un sistema de comunicación entre procesos exclusivamente local (que nunca abandona el nodo modelador). Lo utilizan los programas que necesitan solicitar a `lqosd` que haga algo o recuperar información de él. 
         * La caja `lqos_bus` también actúa como un repositorio para estructuras de datos compartidas cuando los datos se pasan entre partes del programa.
      * `lqos_config` Gestiona la integración de Rust con los archivos de configuración `ispConfig.py` y `/etc/lqos.conf`. Está diseñado como un asistente para que otros sistemas accedan rápidamente a los parámetros de configuración.
      * `lqos_python` se compila en una biblioteca cargable de Python, lo que proporciona una interfaz conveniente para el código Rust desde las partes en Python del programa.
      * `lqos_setup` Proporciona un sistema de configuración inicial basado en texto para usuarios que instalan LibreQoS a través de `apt-get`.
      * `lqos_utils` Proporciona una combinación de funciones útiles que nos han resultado útiles en otras partes del sistema.
* **Python: operación e integración del sistema**
   * `LibreQoS.py` asigna todos los circuitos a los controladores TC y pone en funcionamiento el sistema modelador.
   * `ispConfig.py` proporciona una configuración para todo el sistema.
   * `integrationX.py` proporciona integraciones con UISP, Spylnx y otras herramientas CRM.
   * `lqTools.py` proporciona una interfaz para recopilar estadísticas.

## Lo que estamos construyendo

LibreQoS es un sistema de colas justo, gratuito y de código abierto. Está diseñado para proveedores de servicios de internet (ISP), pero puede ser útil en otros entornos. 

Objetivos principales:

* Proporciona colas justas para ayudar a maximizar el uso de los recursos de Internet que tienes.
* Mantenga baja la latencia del usuario final.
* No altera el tráfico de los usuarios (por ejemplo, reduciendo la calidad de su transmisión de video).
* No invadir la privacidad del usuario.
* Proporcionar excelentes herramientas de soporte para ayudar a mantener su ISP funcionando sin problemas.

Algunos objetivos secundarios incluyen:

* Visualizar datos para facilitar el avance de la tecnología de vanguardia en colas justas.
* Provide amazing throughput on inexpensive hardware.

## Realizar cambios en LibreQoS

Intentamos mantenernos ágiles y con un proceso "ligero".

> Cuando usamos el término "ágil", no nos referimos a un proceso excesivamente formalizado con scrums, tableros kanban y similares. Nos referimos a un proceso ligero, ágil y que se adhiera a guías útiles como "Código Completo" y "El Programador Pragmático". Sin atascarnos en procesos pesados.

### Cambios simples

Para un cambio sencillo, haga lo siguiente:

1. Realice una (o más) de las siguientes acciones:
   * Déjanos saber en el chat que estás trabajando en ello.
   * Crea un asunto (Issue) en nuestro repositorio de GitHub.
   * Envíe una solicitud de extracción (pull request), siguiendo las pautas de la sucursal a continuación.
2. Otros miembros de la comunidad revisan y comentan su cambio de manera informal.
3. Una vez que se llegue al consenso, nos fusionaremos con la rama `develop` (o una rama derivada de `develop` si es un cambio importante) y la probaremos en nuestros recursos de servidor en Equinix.
4. Cuando esté listo, lo fusionaremos en la próxima versión.

### Cambios Complejos

Si necesita o desea un cambio complejo, contáctenos primero a través del chat de Matrix. Queremos evitar duplicar esfuerzos y hacerle perder el tiempo. Estaremos encantados de ofrecerle asesoramiento y orientación. El proceso es similar:
1. Trabaje en su rama local, derivada de `develop`.
2. Crea una solicitud de incorporación de cambios (con el objetivo de "desarrollar"). Si deseas recibir comentarios provisionales, crea un borrador de solicitud de cambio y te ayudaremos a probar tu rama antes de finalizarla.
3. Una vez que tu PR esté listo, envíalo.
4. La comunidad revisará/comentará tu PR.
5. Una vez que se llegue a un consenso, lo fusionaremos en "desarrollar".
6. Una vez listo, `develop` se fusionará con `main`.

## Directrices de las ramas de desarrollo

LibreQoS ha adoptado el siguiente esquema para las ramas de desarrollo:

* `main` - código publicado (etiquetado en releases), listo para su extracción.
* `develop` - Se deriva de `main` y se reestructura en el release. Nada se confirma directamente en `develop`; es el árbol principal para el trabajo de desarrollo continuo.
      * `my_feature` - si está trabajando en una función, su rama de función va aquí, con `develop` como padre. Las solicitudes de incorporación de cambios (PR), una vez que la función esté lista para su inclusión, deben dirigirse a `develop`.
      * `issue_xxx_name`: si estás trabajando en la corrección de un error, trabaja en ello en una rama aquí. Una vez resuelto el problema, las solicitudes de incorporación de cambios deben dirigirse a `develop`.
  * `hotfix_xxx` - Si ocurre una emergencia y necesita enviar una corrección a `main` urgentemente, las ramas de corrección pueden vincularse desde `main`. La solicitud de modificación resultante debe dirigirse a `main`. Informe a los desarrolladores que deben reorganizar `develop`..

El objetivo es que `main` *siempre* sea seguro clonarlo y ejecutarlo, sin sorpresas.

## Directrices del código

Este es un trabajo en progreso.

### Rust

#### Formato de Código

* Usa `cargo fmt` para formatear tu código. Tenemos un formato personalizado.
* Cumplir con las pautas de nombres y casos estándar de Rust.

#### Dependencias

* Comprueba que no estás incluyendo ninguna dependencia que sea incompatible con nuestra licencia---GPL v2.
* Revise los archivos `Cargo.toml` (o ejecute `cargo tree`) e intente preferir usar una dependencia que usemos en otro lugar.
* Intente evitar el uso de cajas sin mantenimiento.

#### Nombramiento

* Don't use short, incomprehensible names in any API or function accessible from outside. You don't save any RAM by naming your variable `sz` instead of `size`---you just make it harder for anyone reading the code.
* It's fine to use `i` and similar for internal counting iterators. Try to use meaningful names for everything else.

#### Estilo de Código

* Prefiere el código funcional/iterativo al imperativo. A veces se necesita un bucle "for", pero si se puede reemplazar con un "iter", un "map" y un "fold", compilará código más rápido y será menos propenso a errores.
* Es mejor tener muchas funciones pequeñas que una sola grande. Rust es muy bueno con la inlineación, y es mucho más fácil entender funciones cortas. También es más fácil probar funciones pequeñas.
* Si tiene que anular una advertencia de Clippy, agregue un comentario explicando por qué lo hizo.
* Las funciones accesibles desde otras cajas deben utilizar el estándar RustDoc para la documentación.

#### Pruebas Unitarias

* Si soluciona un problema y se puede probar: agregue una prueba unitaria para verificar que no retrocedamos y suframos ese error nuevamente.
* Si crea un tipo, escriba pruebas unitarias para probar sus restricciones.

#### Manejo de errores

* Utilice `thiserror` para emitir mensajes de error legibles desde sus funciones.
* Está bien usar `?` y `anyhow` dentro de las cadenas de funciones; prefiera `result.map_err` para transformar sus errores en sus propios errores siempre que sea posible que un error pueda ser devuelto desde una función accesible más allá de la caja inmediata.
* Emite un mensaje `log::error!` o `log::warn!` cuando se produzca un error. No confíes en que el receptor lo haga por ti. Es mejor tener mensajes de error duplicados que ninguno.

