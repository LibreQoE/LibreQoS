# Metodología de QoE y RTT por circuito

## Propósito de esta página

Use esta página para entender cómo LibreQoS calcula actualmente el RTT y el QoE a nivel de circuito en la WebUI.

Esta página describe la metodología enfocada en circuitos usada por las páginas de circuito y otras vistas de experiencia por circuito. Las vistas de sitio, nodo y globales pueden usar rollups diferentes.

## Qué significan estas métricas

- `RTT` es la latencia representativa de ida y vuelta para el circuito.
- `QoE` es la puntuación representativa de calidad para el circuito, basada en latencia y pérdida.
- Ambas métricas buscan reflejar la experiencia general del suscriptor a través de sus destinos activos, en lugar de permitir que un solo flujo defina todo el circuito.

## RTT del circuito

LibreQoS calcula actualmente el RTT del circuito en cuatro etapas:

1. Los flujos activos recientes se agrupan por ASN de destino dentro del circuito.
2. Los flujos muy pequeños se ignoran para el aporte de RTT hasta que hayan transferido al menos `128 KB` en esa dirección.
3. Cada ASN construye su propia vista de RTT a partir del tráfico con RTT observable que LibreQoS realmente puede ver.
4. LibreQoS combina esos valores de RTT por ASN en un único RTT de circuito usando una mediana ponderada.

Esto da más peso a los destinos activos relevantes y al mismo tiempo resiste outliers de un solo flujo o de un solo destino.

## QoE del circuito

El QoE del circuito usa la misma agrupación por ASN que el RTT del circuito.

Para cada ASN activo, LibreQoS:

- construye una vista de RTT a partir del tráfico reciente con RTT observable
- estima la pérdida de transporte usando retransmisiones TCP cuando están disponibles
- aplica el perfil de QoE seleccionado en `qoo_profiles.json`

Después, LibreQoS combina los valores de QoE por ASN en una sola puntuación de QoE del circuito.

## Cómo funciona la ponderación por ASN

LibreQoS no trata todos los flujos por igual.

En cambio, las versiones actuales:

- agrupan los flujos por ASN de destino
- dan más influencia a los ASN que transportan más tráfico activo
- reducen la influencia cuando el tráfico con RTT visible es solo una pequeña parte del tráfico total de ese ASN
- limitan la influencia de un solo ASN para que un único destino no domine por completo la puntuación del circuito cuando existen suficientes ASN distintos activos

La intención es representar mejor la experiencia del suscriptor cuando un circuito habla con muchos destinos al mismo tiempo.

## Por qué esto es mejor que ponderar flujos crudos

Ponderar flujos crudos puede ser engañoso:

- muchos flujos diminutos pueden introducir ruido
- un solo flujo grande de streaming puede exagerar un problema que el ISP no puede influir
- el tráfico dominado por QUIC suele tener visibilidad de RTT más débil que TCP

El método ajustado por ASN reduce esos problemas porque:

- ignora flujos muy pequeños para la contribución al RTT
- pondera por grupos de destino en lugar de por conteo bruto de flujos
- reduce el peso de evidencia débil de RTT
- limita cuánto puede controlar el resultado un solo grupo de destinos

## Límites importantes

Esto sigue siendo una aproximación de la experiencia del usuario, no un clasificador perfecto.

Tenga en cuenta estos límites:

- La visibilidad de RTT es mejor para TCP que para tráfico QUIC cifrado.
- Un solo ASN aún puede representar múltiples aplicaciones con comportamientos distintos.
- En circuitos con muy pocos destinos activos, cualquier límite de influencia por ASN tiene menos margen para funcionar.
- Las métricas de retransmisiones mostradas en otras partes de la WebUI siguen siendo indicadores directos de salud del transporte y no todas están ajustadas por ASN.

## Cómo interpretar el resultado

Use el RTT y el QoE del circuito como señales de experiencia, no como forensia de protocolos.

Ejemplos:

- Si el throughput está sano, la mayoría de destinos se ven bien y un destino de streaming se ve mal, el QoE debería degradarse menos que con un método basado en flujos crudos.
- Si varios destinos activos importantes se ven mal, el RTT y el QoE del circuito deberían seguir reflejándolo claramente.
- Si la cobertura de RTT es débil porque la mayor parte del tráfico es QUIC u otro tráfico opaco, trate la puntuación como direccional y no como absoluta.

## Páginas relacionadas

- [Configurar LibreQoS](configuration-es.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui-es.md)
- [TreeGuard](treeguard-es.md)
