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
