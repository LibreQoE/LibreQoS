# CAKE

Por defecto, LibreQoS utiliza CAKE (Common Applications Kept Enhanced) con el parámetro diffserv4.

## DSCP

[https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml](https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml)

### Clases de tráfico y etiquetas DSCP para Diffserv4

 * Tráfico Sensible a la Latencia  (CS7, CS6, EF, VA, CS5, CS4)
 * Medios de Streaming    (AF4x, AF3x, CS3, AF2x, TOS4, CS2, TOS1)
 * Mejor Esfuerzo        (CS0, AF1x, TOS2, y los no especificados)
 * Tráfico en Segundo Plano (CS1)

### Lista de puntos de código Diffserv conocidos:

 *  Esfuerzo Mínimo (CS1)
 *  Mejor Esfuerzo (CS0)
 *  Máxima Confiabilidad y LLT “Lo” (TOS1)
 *  Máximo Rendimiento (TOS2)
 *  Mínima Demora (TOS4)
 *  LLT "La" (TOS5)
 *  Reenvío Asegurado 1 (AF1x) - x3
 *  Reenvío Asegurado 2 (AF2x) - x3
 *  Reenvío Asegurado 3 (AF3x) - x3
 *  Reenvío Asegurado 4 (AF4x) - x3
 *  Clase de Precedencia 2 (CS2)
 *  Clase de Precedencia 3 (CS3)
 *  Clase de Precedencia 4 (CS4)
 *  Clase de Precedencia 5 (CS5)
 *  Clase de Precedencia 6 (CS6)
 *  Clase de Precedencia 7 (CS7)
 *  Admisión de Voz (VA)
 *  Reenvío Expedido (EF)

### Lista de clases de tráfico según el RFC 4594:

(ordenadas aproximadamente de mayor a menor prioridad en caso de congestión)

(ordenadas aproximadamente de menor a mayor rendimiento en ausencia de congestión)

 *  Control de Red (CS6,CS7)      - tráfico de enrutamiento
 *  Telefonía (EF,VA)         - también conocido como VoIP
 *  Señalización (CS5)               - configuración de llamadas VoIP
 *  Conferencias Multimedia (AF4x) - por ejemplo, videollamadas
 *  Interactividad en Tiempo Real (CS4)     - por ejemplo, videojuegos
 *  Streaming Multimedia (AF3x)    - por ejemplo, YouTube, Netflix, Twitch
 *  Difusión de Video (CS3)
 *  Datos de Baja Latencia (AF2x,TOS4)      - por ejemplo, base de datos
 *  Operación, Administración y Gestión (CS2,TOS1) - por ejemplo, SSH
 *  Servicio Estándar (CS0 & puntos de código no reconocidos)
 *  Datos de Alto Rendimiento (AF1x,TOS2)  - por ejemplo, navegación web
 *  Datos de Baja Prioridad (CS1)           - por ejemplo, BitTorrent