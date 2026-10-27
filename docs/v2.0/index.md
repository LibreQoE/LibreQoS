# LibreQoS v2.0 Documentation (archived)

**Version: LibreQoS v2.0 (archived, v2.0-only).** This documentation set covers
the LibreQoS **v2.0** release. It is retained for reference and for v2.0 hosts
that have not yet upgraded.

**For current releases, use the [LibreQoS v3 documentation](../v3/index.md).**

```{warning}
**Do not follow v2.0 instructions on a v3 host.** LibreQoS v3 removed the kernel
datapath (`tc`/CAKE/HTB + eBPF/XDP attach), the `lqos_bakery` scheduler, the
Python refresh engine, and the Flask `lqos_api`, and it changed the
configuration surface. See [What's new in v3](../v3/whats-new.md) and
[Upgrading from v2.0](../v3/upgrade-from-v2.md).
```

## Getting Started

```{toctree}
:maxdepth: 1

quickstart
design
requirements
prereq
ubuntu-server
bridge
```

## Managing LibreQoS (v1.5 to v2.0)

```{toctree}
:maxdepth: 1

configuration
glossary
operating-modes
best-practices
configuration-advanced
integrations
components
node-manager-ui
scale-topology
stormguard
treeguard
high-availability
recipes
case-studies
future-development-inputs
update
troubleshooting
```

## Integrations

```{toctree}
:maxdepth: 1

integrations-reference
integrations-uisp
integrations-splynx
integrations-netzur
integrations-powercode
integrations-sonar
integrations-visp
integrations-wispgate
```

## Recipes

The recipe sub-pages are indexed from the [Deployment Recipes](recipes.md) page.

## Install, update, and security (v2.0)

```{toctree}
:maxdepth: 1

git-install
https-caddy
radius
```

## Licensed features

```{toctree}
:maxdepth: 1

insight
api
```

## Architecture and tuning

```{toctree}
:maxdepth: 1

libreqos-backend-architecture
topology-data-flow
htb_fq_codel_cake
performance-tuning
```

## Misc

```{toctree}
:maxdepth: 1

extras
```
