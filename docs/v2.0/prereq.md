# Server Setup Prerequisites

## Disable Hyper-Threading
Disable hyperthreading on the BIOS/UEFI of your host system. Hyperthreaading is also known as Simultaneous Multi Threading (SMT) on AMD systems. Disabling this is very important for optimal performance of the XDP cpumap filtering and, in turn, throughput and latency.

- Boot, pressing the appropriate key to enter the BIOS settings
- For AMD systems, you will have to navigate the settings to find the "SMT Control" setting. Usually it is under something like ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Once you find it, switch to "Disabled" or "Off"
- For Intel systems, you will also have to navigate the settings to find the "hyperthrading" toggle option. On HP servers it's under ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options.```
- Save changes and reboot
