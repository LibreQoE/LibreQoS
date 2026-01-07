# Extras

## Flamegraph

```shell
git clone https://github.com/brendangregg/FlameGraph.git
cd FlameGraph
sudo perf record -F 99 -a -g -- sleep 60
perf script > out.perf
./stackcollapse-perf.pl out.perf > out.folded
./flamegraph.pl --title LibreQoS --width 7200 out.folded > libreqos.svg
```

## Cómo habilitar CAKE o fq_codel en enlaces de respaldo (menor capacidad) en redes MikroTik

Configure un Queue Tree en la interfaz del enlace punto a punto de respaldo, en el router MikroTik de cada extremo del enlace.

Para enlaces inferiores a 500 Mbps en hardware razonable como un CCR2116, use CAKE:

```
/queue type
add kind=fq-codel name=fq_codel
add cake-diffserv=diffserv4 kind=cake name=cake
/queue tree
add max-limit=300M name=BACKUP_LINK_INTERFACE packet-mark=no-mark parent=BACKUP_LINK_INTERFACE queue=cake
```

Para enlaces de 500-1000 Mbps, use fq_codel:

```
/queue type
add kind=fq-codel name=fq_codel
add cake-diffserv=diffserv4 kind=cake name=cake
/queue tree
add max-limit=300M name=BACKUP_LINK_INTERFACE packet-mark=no-mark parent=BACKUP_LINK_INTERFACE queue=fq_codel
```
