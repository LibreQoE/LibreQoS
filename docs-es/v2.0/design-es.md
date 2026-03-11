# Escenarios de Despliegue

## Despliegue inline estándar

LibreQoS se coloca en línea en el borde de la red, normalmente entre el router de borde (NAT, firewall) y el router o switch de distribución principal.

#### NAT/CG-NAT
Si utilizas NAT/CG-NAT, coloca LibreQoS en línea antes del punto donde se aplica el NAT, ya que necesita regular las direcciones previas al NAT (100.64.0.0/12) y no las IP públicas posteriores.

#### MPLS/VPLS
LibreQoS puede analizar tráfico MPLS siempre que siga el patrón estándar:
```
(mpls tags)(optional vlan tags)(ip header)
```
Si tu despliegue MPLS usa un patrón distinto, lo ideal es terminar el tráfico MPLS en el router o switch de distribución principal antes de que llegue a LibreQoS.

#### Enrutamiento dinámico: ruta de bypass
- Ruta primaria (bajo costo) *a través* del servidor que ejecuta LibreQoS.
- Ruta de respaldo (alto costo) *evitando* el servidor que ejecuta LibreQoS.

```{mermaid}
flowchart LR
    A[Router de borde] -->|Ruta preferida OSPF/BGP| B[Ruta inline de LibreQoS]
    B --> C[Router/Switch core]
    A -->|Ruta de respaldo de mayor costo| C
```

Interpretación:
1. En operación normal, el enrutamiento prefiere la ruta inline de bajo costo a través de LibreQoS.
2. Si la ruta inline falla, el enrutamiento converge hacia la ruta bypass de mayor costo.
3. Tras la recuperación, la preferencia de rutas devuelve el tráfico a la ruta inline.

### Opción 1: Enrutamiento dinámico (recomendado)

Recomendamos usar protocolos como OSPF para definir rutas de bajo y alto costo entre el router de borde y el router/switch de distribución. La ruta de bajo costo debe pasar “a través” de las interfaces puente del shaper para que LibreQoS pueda observar y regular el tráfico. Por ejemplo, la ruta OSPF de bajo costo puede tener costo 1. El enlace de alto costo (respaldo) debe omitir por completo LibreQoS y configurarse con un costo mayor (por ejemplo 100 en OSPF) para que el tráfico solo tome ese camino cuando el puente de LibreQoS no esté operativo.

## Despliegue de laboratorio (opcional)
Cuando pruebes LibreQoS por primera vez puedes desplegar un laboratorio a pequeña escala para verlo en acción.
![image](https://github.com/user-attachments/assets/6174bd29-112d-4b00-bea8-41314983d37a)
