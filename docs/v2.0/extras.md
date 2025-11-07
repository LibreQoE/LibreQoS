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

## How to enable CAKE or fq_codel on backup (lower capacity) backhaul links on MikroTik Networks

Set a Queue Tree on the backup point-to-point link's interface, on the Mikrotik router on each end of a link.

For <500M links on reasonable hardware such as a CCR2116, use CAKE:

```
/queue type
add kind=fq-codel name=fq_codel
add cake-diffserv=diffserv4 kind=cake name=cake
/queue tree
add max-limit=300M name=BACKUP_LINK_INTERFACE packet-mark=no-mark parent=BACKUP_LINK_INTERFACE queue=cake
```

For 500-1000M links, use fq_codel:

```
/queue type
add kind=fq-codel name=fq_codel
add cake-diffserv=diffserv4 kind=cake name=cake
/queue tree
add max-limit=300M name=BACKUP_LINK_INTERFACE packet-mark=no-mark parent=BACKUP_LINK_INTERFACE queue=fq_codel
```
