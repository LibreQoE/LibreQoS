# Receta: Fabric Switch-Centric con Bypass de Mantenimiento (incluye SD-WAN)

Use este patron cuando el core de red es switch-centric y basado en VLANs, y necesita rutas primarias con shaping mas rutas bypass para mantenimiento sin downtime.

## Ajuste

- Mejor para: fabrics con ingenieria L2/L3 explicita.
- Evitar cuando: no hay ownership claro de rutas y parent mapping.

## Patron Topologico

VLANs primarias (con shaping via LibreQoS):

1. `EdgeA <-> CoreA`
2. `EdgeA <-> CoreB`
3. `EdgeB <-> CoreA`
4. `EdgeB <-> CoreB`

Deben existir VLANs bypass, con politica de routing que prefiera la ruta con shaping en estado normal.

## Ilustraciones Topologicas

### Conjunto de VLANs con Shaping

```{mermaid}
flowchart LR
    EA[Router EdgeA]
    EB[Router EdgeB]
    SW[Par de Switches SW1/SW2]
    LQ[LibreQoS Bridge]
    CA[Router CoreA]
    CB[Router CoreB]

    EA --> SW
    EB --> SW
    SW -->|VLANs Shaped 110,120,210,220| LQ
    LQ --> SW
    SW --> CA
    SW --> CB
```

Mapeo shaped:

- VLAN 110: `EdgeA <-> CoreA` (via LibreQoS)
- VLAN 120: `EdgeA <-> CoreB` (via LibreQoS)
- VLAN 210: `EdgeB <-> CoreA` (via LibreQoS)
- VLAN 220: `EdgeB <-> CoreB` (via LibreQoS)

### Conjunto de VLANs Bypass

```{mermaid}
flowchart LR
    EA[Router EdgeA]
    EB[Router EdgeB]
    SW[Par de Switches SW1/SW2]
    CA[Router CoreA]
    CB[Router CoreB]

    EA -.-> SW
    EB -.-> SW
    SW -. VLANs Bypass 310,320,410,420 .-> CA
    SW -. VLANs Bypass 310,320,410,420 .-> CB
```

Mapeo bypass:

- VLAN 310: `EdgeA <-> CoreA`
- VLAN 320: `EdgeA <-> CoreB`
- VLAN 410: `EdgeB <-> CoreA`
- VLAN 420: `EdgeB <-> CoreB`

## Ejemplo MikroTik RouterOS v7 (OSPF conceptual)

```text
/routing ospf instance
add name=default-v2 router-id=10.255.255.1

/routing ospf area
add name=backbone-v2 area-id=0.0.0.0 instance=default-v2

/routing ospf interface-template
add interfaces=vlan-edgea-corea-lq area=backbone-v2 cost=10
add interfaces=vlan-edgea-coreb-lq area=backbone-v2 cost=10
add interfaces=vlan-edgeb-corea-lq area=backbone-v2 cost=10
add interfaces=vlan-edgeb-coreb-lq area=backbone-v2 cost=10
add interfaces=vlan-edgea-corea-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgea-coreb-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgeb-corea-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgeb-coreb-bypass area=backbone-v2 cost=200
```

## Implementacion

1. Construya y verifique VLANs primarias y bypass.
2. Mantenga preferencia de ruta deterministica (OSPF/BGP).
3. Coloque LibreQoS inline en rutas primarias.
4. Valide failover/failback en ventana de mantenimiento.

## Checklist de Validacion

1. Estado normal: trafico en ruta primaria con shaping.
2. Falla o mantenimiento: convergencia hacia bypass.
3. Recuperacion: retorno estable a ruta primaria.
4. Salud de WebUI estable durante transiciones.

## Variante SD-WAN

Para SD-WAN use el mismo modelo:

- Underlay primario pasando por ruta LibreQoS.
- Underlay secundario como bypass de mantenimiento/falla.
- Nombres de nodos y relaciones parent estables.

## Rollback

1. Fuerce preferencia temporal a bypass.
2. Restaure politica previa de ruta LibreQoS.
3. Rehabilite preferencia de ruta primary tras verificar.

## Paginas Relacionadas

- [Alta Disponibilidad y Dominios de Falla](high-availability-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
- [Configurar Bridge de Shaping](bridge-es.md)
- [Troubleshooting](troubleshooting-es.md)
