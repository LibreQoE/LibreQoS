# Receta: Despliegue Proxmox VM con 3 NICs

Use este patron para despliegues VM donde LibreQoS corre en Ubuntu Server 24.04 con interfaces dedicadas de gestion y shaping.

## Ajuste

- Mejor para: operadores estandarizados en Proxmox.
- Evitar cuando: objetivos de throughput/latencia requieren mayor headroom de bare metal.

## Seleccion de Patron

- Patron A (recomendado): bridges dedicados para shaping (`vmbr1`, `vmbr2`)
- Patron B (alternativo): bridge unico (`vmbr0`) con virtio NICs etiquetadas por VLAN

## Roles de Interfaces

- `ens18`: gestion (IP asignada)
- `ens19`: shaping puerto 1
- `ens20`: shaping puerto 2

`ens19` y `ens20` son interfaces de shaping para LibreQoS (`to_internet`/`to_network`).
`ens18`/`ens19`/`ens20` son nombres de ejemplo; verifique nombres reales en su VM.

## Patron A (Recomendado): Bridges de Shaping Dedicados

Intencion del host:

- `vmbr1` respaldado por ruta de shaping A en el host.
- `vmbr2` respaldado por ruta de shaping B en el host.
- Gestion normalmente sin tag en `vmbr0`.

Referencia de mapeo host->guest:

| Puerto/ruta host Proxmox | Bridge | vNIC VM | NIC guest | Rol LibreQoS |
|---|---|---|---|---|
| Uplink gestion (ejemplo `eno1`) | `vmbr0` | `net0` | `ens18` | Gestion |
| Ruta shaping A (ejemplo `eno2`) | `vmbr1` | `net1` | `ens19` | `to_internet` o `to_network` |
| Ruta shaping B (ejemplo `eno3`) | `vmbr2` | `net2` | `ens20` | opuesto de `ens19` |

Ejemplo en Proxmox UI:

- `net0`: `virtio`, `bridge=vmbr0` (gestion)
- `net1`: `virtio`, `bridge=vmbr1`, `multiqueue=<cantidad vCPU>` (shaping A)
- `net2`: `virtio`, `bridge=vmbr2`, `multiqueue=<cantidad vCPU>` (shaping B)

```{mermaid}
flowchart LR
    subgraph HOST[Host Proxmox]
        H0[vmbr0 (gestion untagged)]
        H1[vmbr1 (ruta shaping A)]
        H2[vmbr2 (ruta shaping B)]
    end

    subgraph VM[LibreQoS VM]
        N0[net0 virtio]
        N1[net1 virtio]
        N2[net2 virtio]
        E18[ens18 gestion]
        E19[ens19 shaping A]
        E20[ens20 shaping B]
    end

    H0 --> N0 --> E18
    H1 --> N1 --> E19
    H2 --> N2 --> E20
```

## Patron B (Alternativo): Bridge Unico con NICs Taggeadas

Intencion del host:

- `vmbr0` lleva trunk de VLANs de gestion y shaping.
- Gestion normalmente taggeada en este patron.

Ejemplo en Proxmox UI:

- `net0`: `virtio`, `bridge=vmbr0`, `tag=99` (gestion ejemplo)
- `net1`: `virtio`, `bridge=vmbr0`, `tag=110`, `multiqueue=<cantidad vCPU>` (shaping A)
- `net2`: `virtio`, `bridge=vmbr0`, `tag=120`, `multiqueue=<cantidad vCPU>` (shaping B)

Notas:

- Los VLAN IDs son ejemplos; use los de su diseno.
- Con tag en Proxmox NIC, normalmente el trafico llega sin tag dentro del guest NIC.

```{mermaid}
flowchart LR
    subgraph HOST[Host Proxmox]
        T0[vmbr0 trunk VLAN]
    end

    subgraph VM[LibreQoS VM]
        PN0[net0 virtio tag 99]
        PN1[net1 virtio tag 110]
        PN2[net2 virtio tag 120]
        PE18[ens18 gestion]
        PE19[ens19 shaping A]
        PE20[ens20 shaping B]
    end

    T0 --> PN0 --> PE18
    T0 --> PN1 --> PE19
    T0 --> PN2 --> PE20
```

## Prerrequisitos

1. Revise [Prerequisites](prereq-es.md) y [System Requirements](requirements-es.md).
2. Habilite multiqueue en vNICs de shaping y ajuste al numero de vCPUs.
3. Para >10 Gbps, use passthrough cuando corresponda.

## Patron Netplan

```yaml
network:
  version: 2
  ethernets:
    ens18:
      addresses:
        - 100.99.0.10/24
      routes:
        - to: default
          via: 100.99.0.1
      nameservers:
        addresses: [1.1.1.1, 8.8.8.8]
    ens19:
      dhcp4: no
      dhcp6: no
    ens20:
      dhcp4: no
      dhcp6: no
```

Luego configure el comportamiento bridge segun [Configure Shaping Bridge](bridge-es.md).

## Checklist de Validacion

1. Confirme que la VM usa Patron A o B.
2. Patron A: confirme `net1->vmbr1` y `net2->vmbr2`.
3. Patron B: confirme `net1->vmbr0 tag 110` y `net2->vmbr0 tag 120` (o sus tags).
4. Confirme `to_internet` / `to_network` en `ens19`/`ens20`.
5. Confirme que `ens19` y `ens20` no tienen IP en Netplan guest.
6. Confirme salud de scheduler y daemon.
7. Confirme throughput/latencia esperados para envelope VM.
8. Confirme ausencia de shaping asimetrico.

## Rollback

1. Mueva trafico a ruta previa o bypass.
2. Revierta configuracion de interfaces/colas de VM.
3. Reinicie servicios y revalide.

## Paginas Relacionadas

- [Prerequisites](prereq-es.md)
- [System Requirements](requirements-es.md)
- [Configure Shaping Bridge](bridge-es.md)
- [Troubleshooting](troubleshooting-es.md)
