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

#### Diagrama
![Offical Configuration](https://github.com/user-attachments/assets/e5914a58-3ec6-4eb1-b016-8a57582dd082)

### Opción 1: Enrutamiento dinámico (recomendado)

Recomendamos usar protocolos como OSPF para definir rutas de bajo y alto costo entre el router de borde y el router/switch de distribución. La ruta de bajo costo debe pasar “a través” de las interfaces puente del shaper para que LibreQoS pueda observar y regular el tráfico. Por ejemplo, la ruta OSPF de bajo costo puede tener costo 1. El enlace de alto costo (respaldo) debe omitir por completo LibreQoS y configurarse con un costo mayor (por ejemplo 100 en OSPF) para que el tráfico solo tome ese camino cuando el puente de LibreQoS no esté operativo.

### Opción 2: Spanning Tree Protocol (no recomendado)

También puedes usar Spanning Tree Protocol con costos de ruta si OSPF u otro protocolo dinámico no es posible.

```{note}
Casi las mismas consideraciones aplican a la configuración alternativa que a la configuración oficialmente soportada.
```

Ten en cuenta que, si usas enlaces con distintos anchos de banda (por ejemplo, 10 Gbps a través de LibreQoS y 1 Gbps entre el switch central y el router de borde), deberás ajustar con mayor cuidado los costos de STP.

## Despliegue de laboratorio (opcional)
Cuando pruebes LibreQoS por primera vez puedes desplegar un laboratorio a pequeña escala para verlo en acción.
![image](https://github.com/user-attachments/assets/6174bd29-112d-4b00-bea8-41314983d37a)
