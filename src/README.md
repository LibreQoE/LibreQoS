# v1.4 (Alpha)

![image](https://i0.wp.com/libreqos.io/wp-content/uploads/2023/01/v1.4-alpha-2.png?w=3664&ssl=1)

See [wiki here](https://github.com/LibreQoE/LibreQoS/wiki/v1.4)

## Planner (binpacking replacement)

LibreQoS now uses an internal planner for assigning circuits and nodes to CPU queues and generated parent nodes. The planner favors stable, stateful assignments to minimize churn and reduce rebuilds, replacing the previous external binpacking dependency.
