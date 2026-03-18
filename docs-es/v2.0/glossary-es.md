# Glosario

Use esta página para definiciones consistentes usadas en la documentación operativa de LibreQoS.

## Fuente de verdad

El sistema que es dueño de los datos durables de shaping y debe tratarse como autoritativo.

## Ciclo de refresco de integración

Proceso periódico de sincronización que importa datos del CRM/NMS y regenera insumos de shaping.

## Override

Ajuste específico que se aplica por encima de los datos base importados/manuales.

## Cambio persistente

Cambio que permanece entre ciclos de refresco/reinicio, salvo eliminación explícita.

## Cambio transitorio

Cambio temporal que puede ser reemplazado por sincronizaciones o refrescos posteriores.

## Sobrescribible por integración

Datos que la salida de sincronización de integración puede reemplazar en operación normal.

## ShapedDevices.csv

Archivo de entrada de shaping de suscriptores/dispositivos usado por el scheduler.

## network.json

Archivo de entrada de topología y capacidad de nodos usado para jerarquía y estructura de shaping.

## Circuito mapeado

Circuito actualmente resuelto/mapeado dentro del estado activo de shaping.

## Límite de circuitos mapeados

Tope aplicado a circuitos mapeados según estado actual de licencia/política.

## Estado del scheduler

Estado operativo de `lqos_scheduler` expuesto por WebUI/API.

## Impacto inmediato en runtime

Acción de API/UI/CLI que puede afectar el shaping activo poco después de ejecutarse.
