# Suposiciones de Diseño de Red

## Configuración oficialmente soportada

- LibreQoS se coloca en línea en el borde de su red, normalmente entre el router de borde (edge router) de la red (NAT, firewall) y el router/switch de distribución central (core router/switch).

![Offical Configuration](https://github.com/LuisDanielEA/LibreQoS/blob/develop/docs-es/v2.0/design-images/Normal.png)

### Rutas Primaria y de Respaldo

Recomendamos usar protocolos de enrutamiento dinámico como OSPF para crear una ruta de bajo costo y otra de alto costo entre el router de borde y el router/switch de distribución central. The low cost path should pass "through" the LibreQoS shaper bridge interfaces to allow LibreQoS to observe and shape traffic.
La ruta de bajo costo debe pasar a través de las interfaces del puente regulador de LibreQoS para permitir que LibreQoS observe y regule el tráfico. 
Por ejemplo, una ruta OSPF de bajo costo puede establecerse con un valor de 1. El enlace de alto costo (respaldo) pasaría completamente por fuera de LibreQoS, configurándose con un costo mayor (quizás 100 en OSPF) para asegurar que el tráfico solo tome ese camino cuando el puente regulador de LibreQoS no esté funcionando.

### NAT/CG-NAT
Si utiliza NAT/CG-NAT, coloque LibreQoS en línea antes de donde se aplica el NAT, ya que LibreQoS necesita regular las direcciones pre-NAT (100.64.0.0/12), no las IPs públicas post-NAT.

### MPLS/VPLS
LibreQoS puede trabajar con tráfico MPLS, sin embargo, el tráfico debe seguir el patrón estándar:
```
(mpls tags)(optional vlan tags)(ip header)
```
Si utiliza MPLS con un patrón de etiquetas diferente, lo ideal es terminar el tráfico MPLS en el router/switch de distribución central (core router) antes de que llegue a LibreQoS.

## Configuración de laboratorio de pruebas
Cuando pruebe LibreQoS por primera vez, recomendamos desplegar un laboratorio de pruebas a pequeña escala para verlo en acción.
![image](https://github.com/LuisDanielEA/LibreQoS/blob/develop/docs-es/v2.0/design-images/Testbed.png)

### Network Interface Card

```{note}
Debe contar con una de estas opciones:
- Una sola NIC con dos interfaces,  
- Dos NICs con una sola interfaz cada una,  
- 2x interfaces VLAN (usando una o dos NICs).  
```

LibreQoS requiere que las NICs tengan 2 o más filas RX/TX y soporte para XDP.
Aunque muchas tarjetas cumplen teóricamente con estos requisitos, las tarjetas menos comunes tienden a tener errores de controlador no reportados que afectan la funcionalidad de XDP y las hacen inutilizables para nuestros fines.
En este momento recomendamos las NICs Intel x520, Intel x710 y Nvidia (ConnectX-5 o más reciente). No podemos garantizar compatibilidad con otras tarjetas.

## Configuración Alternativa

Esta configuración alternativa utiliza Spanning Tree Protocol (STP) para modificar la ruta de datos en caso de que el dispositivo LibreQoS esté fuera de línea por mantenimiento u otro problema.

```{note}
La mayoría de las consideraciones aplican tanto para la configuración alternativa como para la oficialmente soportada.
```

- LibreQoS se coloca en línea en el borde de su red, normalmente entre el router de borde (edge router) de la red (NAT, firewall) y el router/switch de distribución central (core router/switch).
- Si utiliza NAT/CG-NAT, coloque LibreQoS en línea antes de donde se aplica el NAT, ya que LibreQoS necesita regular las direcciones pre-NAT (100.64.0.0/12), no las IPs públicas post-NAT.
- Para redes que usan MPLS: LibreQoS puede trabajar con tráfico MPLS, pero este debe seguir el patrón estándar (mpls tags)(optional vlan tags)(ip header). Si utiliza un patrón diferente, lo ideal es terminar el tráfico MPLS en el router/switch de distribución central (core router/switch) antes de que llegue a LibreQoS.
- Enlace primario de Spanning Tree (bajo costo) a través del servidor que ejecuta LibreQoS.
- Enlace de respaldo de Spanning Tree (alto costo, por ejemplo 80).

Keep in mind that if you use different bandwidth links, for example, 10 Gbps through LibreQoS, and 1 Gbps between core switch and edge router, you may need to be more intentional with your STP costs.
Tenga en cuenta que si utiliza enlaces de diferente capacidad, por ejemplo, 10 Gbps a través de LibreQoS y 1 Gbps entre el switch central (core switch) y el router de borde (edge router), puede que necesite ser más enfático con los costos de STP.

![image](https://github.com/LuisDanielEA/LibreQoS/blob/develop/docs-es/v2.0/design-images/Alternate.png)
