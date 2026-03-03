# Receta: Hoteleria y Hospitalidad con Shaping por Dispositivo

Use este patron cuando cada dispositivo cliente debe recibir comportamiento de circuito propio.

## Ajuste

- Mejor para: entornos de hospitalidad con pools de direcciones previsibles y objetivos de equidad por dispositivo.
- Evitar cuando: el conteo proyectado de circuitos por dispositivo supera limites practicos de RAM/colas.

## Guardrails de Capacidad

El shaping por dispositivo eleva rapidamente la cantidad de circuitos. Valide:

1. RAM segun volumen esperado ([Requisitos del Sistema](requirements-es.md)).
2. Presion de queue/class y estado de urgencias bajo carga realista.
3. Impacto de CAKE qdisc (memoria y overhead operativo).
4. Si pruebas pico muestran presion persistente o crecimiento de memoria inseguro, migre a agrupacion por habitacion o subred.

## Patron

- Construya una lista enumerada de IPv4 posibles por dispositivo.
- Asigne un circuito por IP de dispositivo.
- Use grupos de parent estables (piso/edificio/ala).

## Ejemplo

```text
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm
HTL-ROOM-1204,Room1204-Device,HTL-ROOM-1204,Room1204-DeviceA,Floor12,,100.70.12.44,,2,2,50,20,Hospitality per-device plan,cake
```

## Checklist de Validacion

1. Mapeo dispositivo-circuito correcto y estable.
2. Sin urgencias persistentes de limites queue/class.
3. Memoria dentro de envelope esperado en ocupacion pico.
4. RTT/retransmisiones aceptables en horas de mayor contencion.

## Rollback

1. Pase de per-device a per-room o per-subnet.
2. Reduzca conteo de circuitos y recargue.
3. Revalide salud de scheduler y presion de colas.

## Paginas Relacionadas

- [Requisitos del Sistema](requirements-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
- [HTB + fq-codel / CAKE](htb_fq_codel_cake-es.md)
- [Troubleshooting](troubleshooting-es.md)
