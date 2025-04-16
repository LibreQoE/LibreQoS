# LibreQoS Tornado

**WARNING**: This is extremely experimental. Don't try this on anyone you like.

> The name is a bit of a joke, and will change. I kept thinking people said "autorotate", and decided to name it "Tornado" because it was a tornado of autorotate. I don't know why I thought that, but it stuck.

LibreQoS Tornado. Automatic top-level HTB rate adjustment, based on capacity monitoring.

Heavily inspired by LynxTheCat's Cake AutoRate project. https://github.com/lynxthecat/cake-autorate

## Usage

Add the following to your `lqos.conf`:

```toml
[tornado]
enabled = true
targets = [ "SITENAME" ]
dry_run = true
```

You can list as many sites as you want in the `targets` array. I strongly recommend `dry_run` for now, which just
emits what it *would* have done to the console!

## How it works

Tornado watches throughput, TCP retransmits and round-trip time going through each target site. 
It currently only uses retransmits --- more is coming. If retransmits are worsening, it slows
the site down. If they are improving (by 20% or so), it speeds the site up. There's a cooldown period between
changes to reduce oscillation.