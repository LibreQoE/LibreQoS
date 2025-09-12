# Configuración de LibreQoS

Esta es la herramienta de configuración de LibreQoS, diseñada para ejecutarse automáticamente como parte del proceso de instalación con `dpkg`. Su objetivo principal es ayudar a los usuarios a establecer rápidamente una configuración mínima funcional de LibreQoS, asegurando que el sistema esté listo para operar con la menor intervención manual posible.

## Propósito

La herramienta de configuración de LibreQoS guía a los usuarios a través de los pasos esenciales necesarios para poner LibreQoS en funcionamiento. Simplifica el proceso inicial de configuramiento al proporcionar una interfaz guiada y automatiza la creación de los archivos de configuración necesarios.

## Qué Configura la Herramienta

La herramienta ayuda a configurar los siguientes componentes:

- **Modo Puente (Bridge)**: Seleccione entre los modos Linux, XDP o Puente Único para adaptarse a su escenario de implementación.
- **Interfaces de Red**: Eliga y configure las interfaces de red que LibreQoS gestionará.
- **Parámetros de Ancho de Banda**: Establezca límites y parámetros de ancho de banda para su red.
- **Rangos de IP**: Defina los rangos de direcciones IP que LibreQoS debe monitorear y regular.
- **Usuarios Web**: Cree y gestione cuentas de usuario para acceder a la interfaz web de LibreQoS.
- **Archivos de Configuración**: Genera y actualiza automáticamente los principales archivos de configuración, incluyendo:
  - `lqos.conf`
  - `network.json`
  - `ShapedDevices.csv`

## Interfaz de Usuario

La herramienta de configuración proporciona una Interfaz de Usuario en Texto construida con la librería [Cursive](https://github.com/gyscos/cursive). Esta interfaz permite a los usuarios navegar de manera interactiva por las opciones de configuración en la terminal, haciendo que el proceso de instalación sea sencillo y amigable.

## Integración con LibreQoS

Al ejecutar esta herramienta durante la instalación, LibreQoS garantiza que todas las configuraciones críticas estén definidas y que el sistema esté listo para su uso inmediato. La herramienta de configuración es una parte integral del sistema LibreQoS, agilizando la implementación y reduciendo el riesgo de errores de configuración.
