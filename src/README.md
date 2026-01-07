# v1.4 (Alpha)

![image](https://i0.wp.com/libreqos.io/wp-content/uploads/2023/01/v1.4-alpha-2.png?w=3664&ssl=1)

See [wiki here](https://github.com/LibreQoE/LibreQoS/wiki/v1.4)

## Binpacking

LibreQoS assigns circuits and sites to CPU queues using a simple, deterministic greedy binpacking approach. We continue to use Insight (lts) to provide weights when available, but assignments themselves are computed on each run without keeping long‑term state.
