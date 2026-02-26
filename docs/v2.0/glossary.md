# Glossary

Use this page for consistent definitions used across LibreQoS operator docs.

## Source of truth

The system that owns durable shaping data and should be treated as authoritative.

## Integration refresh cycle

The recurring sync process that imports CRM/NMS data and regenerates shaping inputs.

## Override

A targeted adjustment layered on top of base imported/manual data.

## Persistent change

A change that remains across refresh/restart cycles unless explicitly removed.

## Transient change

A temporary change that may be replaced by later syncs or file refreshes.

## Integration-overwritable

Data that can be replaced by integration sync output in normal operation.

## ShapedDevices.csv

Subscriber/device shaping input file used by scheduler workflows.

## network.json

Topology and node-capacity input file used for hierarchy and shaping structure.

## Mapped circuit

A circuit currently resolved/mapped into active shaping state.

## Mapped circuit limit

The enforced cap on mapped circuits based on current licensing/policy state.

## Scheduler status

Operational state of `lqos_scheduler` as exposed in WebUI/API.

## Immediate runtime impact

An API/UI/CLI action that can affect active shaping behavior shortly after execution.
