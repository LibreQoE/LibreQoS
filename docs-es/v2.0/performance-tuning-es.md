# Ajuste de Rendimiento

## Ubuntu inicia lentamente (~2 minutos)

### Listar todos los servicios que dependen de la red

```shell
systemctl show -p WantedBy network-online.target
```

### En Ubuntu 22.04 este comando puede ayudar

```shell
systemctl disable cloud-config iscsid cloud-final
```

### Establecer el governor correcto para la CPU (baremetal/hipervisor)

```shell
cpupower frequency-set --governor performance
```

### OSPF

Se recomienda ajustar los temporizadores OSPF de ambos vecinos (core y edge router) para minimizar el tiempo de inactividad cuando el servidor LibreQoS se reinicie.

* hello interval
* dead interval
