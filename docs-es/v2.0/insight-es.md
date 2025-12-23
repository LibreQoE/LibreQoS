# LibreQoS Insight (Insight)

## Acerca de Insight
Conoce más sobre Insight en nuestro sitio web, [aquí](https://libreqos.io/insight/).

## Interfaz de Insight

### Barra de tareas

<img width="355" height="871" alt="taskbar" src="https://github.com/user-attachments/assets/796cba7b-49d0-4a49-96a5-cd12823a6bd8" />

Si tienes más de una caja reguladora (shaper), puedes elegir cuál visualizar desde el desplegable **Shaper Nodes** (ubicado justo debajo del cuadro de búsqueda).

### Selector de tiempo

<img width="529" height="83" alt="image" src="https://github.com/user-attachments/assets/b706a230-883a-4064-9436-2e82749eb8b7" />

Al hacer clic en el selector de tiempo (por defecto Last 24 Hours) puedes definir el rango que se mostrará, desde 15 minutos hasta 28 días. También es posible establecer periodos personalizados (Now minus X minutes) o ventanas muy específicas.

### Panel (Dashboard)

Puedes ver qué caja reguladora se está consultando en la parte superior izquierda, junto a “Select a destination”.

El panel de LibreQoS Insight está basado en widgets y dispone de varias pestañas que se pueden editar.

#### Pestaña Traffic Overview

<img width="3828" height="2160" alt="01 dashboard" src="https://github.com/user-attachments/assets/29ee98e3-55c7-4466-a444-de9542cc0940" />

La pestaña predeterminada es **Traffic Overview**. Muestra el tráfico en vivo y los endpoints con mayor consumo, tanto por ASN como por nodo.

#### Pestaña Shaper

<img width="3840" height="2160" alt="02 shaper tab" src="https://github.com/user-attachments/assets/721fd195-35ad-421b-8d0a-e2aa6e5cf7e9" />

La pestaña **Shaper** muestra estadísticas de alto nivel para una caja LibreQoS determinada.

- **Active Circuit Count:** número de circuitos activos detectados (basado en el tráfico del suscriptor).
- **Throughput:** tráfico agregado del shaper.
- **Shaper Packets:** tasa de paquetes por segundo a lo largo del tiempo.
- **Shaper TCP Retransmits Percentage:** porcentaje de paquetes TCP retransmitidos (proxy de pérdida); idealmente debe permanecer por debajo del 1 %.
- **Shaper CAKE Activity:** nivel de actividad de los shapers CAKE en la red.
- **Shaper Round Trip Time:** RTT promedio del tráfico.
- **Shaper Round Trip Time Histogram:** el mismo RTT en formato histograma.
- **Shaper CPU Utilization:** uso máximo y promedio de CPU en el tiempo.
- **Shaper Memory Utilization:** uso de RAM del shaper en el tiempo.

#### Pestaña Children

<img width="3840" height="2160" alt="03 child view" src="https://github.com/user-attachments/assets/1f0a0e3c-672b-4982-b334-60a681248b99" />

- **Shaper Child Throughput:** rendimiento de cada nodo hijo de primer nivel.
- **Shaper Child TCP Retransmits:** ayuda a detectar nodos con pérdidas elevadas.
- **Shaper Child Round Trip Time:** ayuda a identificar nodos con RTT anómalo.

#### Heatmaps

<img width="3840" height="2160" alt="04 heatmap" src="https://github.com/user-attachments/assets/f3911ca3-8157-43dc-9f57-1402a6cd0204" />

Esta vista muestra heatmaps de RTT, retransmisiones y capacidad para los nodos de primer nivel del shaper.

#### Site Rankings

<img width="3840" height="2160" alt="05 health" src="https://github.com/user-attachments/assets/37e69c41-646c-458f-912d-8556acace102" />

Esta pestaña resume la salud de los sitios / AP / etc. según RTT, retransmisiones TCP y capacidad en cada dirección.

### Node Map

<img width="3840" height="2160" alt="06 map" src="https://github.com/user-attachments/assets/219d75b3-739a-4f41-86e6-5bc270f22afd" />

Permite identificar la topología general de la red desde la perspectiva de Insight. Al pasar el cursor por los enlaces se muestra el throughput y las retransmisiones TCP actuales de ese tramo.

### Libby (Asistente IA)

<img width="1784" height="882" alt="07 libby" src="https://github.com/user-attachments/assets/591a3fd1-3946-44ed-a4fd-e1b1d84b9ef6" />

Libby es el asistente IA de Insight. Puede ayudarte con dudas sobre LibreQoS o Insight basándose en la documentación oficial. Libby accede a datos en vivo y de largo plazo tanto de los shapers sincronizados como de Insight. Puede responder en la mayoría de los idiomas mediante traducción automática.

Si encuentras nuevos casos de uso para Libby, comparte con nosotros cómo te ayuda en el flujo de trabajo.

### Site Heatmap

<img width="3830" height="2160" alt="08 heatmap" src="https://github.com/user-attachments/assets/dfaea245-3221-4cea-874b-fd795ac8da33" />

Esta vista entrega heatmaps de RTT, retransmisiones y capacidad para cada Access Point, OLT y sitio de tu red en una sola pantalla, lo que facilita detectar puntos problemáticos rápidamente.

### Tree History

<img width="3840" height="2160" alt="09 tree history" src="https://github.com/user-attachments/assets/ea0d0417-c937-41ee-ac4d-7d84c162c6dd" />

Se basa en la vista Tree Overview de la WebUI de LibreQoS y muestra el diagrama Sankey a lo largo del tiempo para identificar cuellos de botella que afecten el rendimiento.

### Reports

<img width="3828" height="2160" alt="10 report" src="https://github.com/user-attachments/assets/37f6bd91-8937-4755-a095-6bc38822f544" />

Insight permite generar reportes asistidos por IA sobre circuitos específicos. Estos informes usan los últimos 7 días de actividad del cliente (características por ASN, contexto de topología y contexto geográfico) e incluyen un Perfil de usuario, Hallazgos clave, Problemas críticos, Tendencias de rendimiento, una Recomendación de mejora y elementos sugeridos para revisión manual.

### Alerts

<img width="3831" height="2160" alt="11 alerts" src="https://github.com/user-attachments/assets/66f0a465-eb00-4bfb-9cf8-c8302af78ead" />

La sección **Alerts** entrega advertencias automáticas sobre comportamientos fuera de norma para nodos de la red (AP, OLT, sitios, etc.).
