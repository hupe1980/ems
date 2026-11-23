+++
title = "S2 flexibility and external managers"
description = "Connect a manager, inspect an asset’s flexibility and follow an instruction through execution."
weight = 10

[extra]
group = "Control and metering"
+++

S2 is an energy flexibility protocol. A **Resource Manager** describes an asset and its capabilities. A **Customer Energy Manager** chooses a supported control type and sends instructions for that resource. A household might let an aggregator schedule battery use while retaining control over grid limits and local overrides.

## The five control types

| Control type | What the manager can describe | Household example |
|---|---|---|
| FRBC · Fill Rate Based Control | A store’s fill level and the rates at which it can change | Battery or hot-water store |
| PEBC · Power Envelope Based Control | A permitted power range over time | Inverter curtailment or EV charging |
| OMBC · Operation Mode Based Control | A choice between discrete operating modes | SG Ready heat pump |
| PPBC · Power Profile Based Control | When a programme with a known power profile should start | Dishwasher |
| DDBC · Demand Driven Based Control | Actuators meeting a changing demand | Heat pump serving a reported demand |

What a site can describe depends on its measurements and installed assets. A charger with a known connected car and energy deadline has information that an empty charging port cannot supply. Read the offered control types and execution status for the particular resource.

## Connect a manager

Enable the S2 surface with `[s2] enabled = true` in the gateway configuration. Create a named manager credential with the household credential. Use the gateway certificate verified during commissioning.

```sh
curl --cacert hems.pem -X PUT -H "Authorization: Bearer $HEMS_TOKEN"   https://hems.local:8080/v1/managers/aggregator
```

Store the returned token in the manager. Use it to connect to `wss://hems.local:8080/s2/{asset}`, replacing `{asset}` with the configured asset ID. Each connection negotiates one resource. The household can list managers, rotate a named token or withdraw access. [Credential and route reference](@/docs/api.md).

## Follow an instruction

The manager negotiates the resource description and control type before submitting an instruction. A future execution time queues the request. An acceptance acknowledges that the gateway received it. Follow the subsequent instruction status and device measurements to establish what happened.

The guard enforces site limits on each tick. A household override also has priority over the manager. A superseded or undeliverable instruction can report `ABORTED`; resuming requires a new instruction. For example, the virtual battery can report its current fill level while aborting a power request because its pack controls itself. A manager should record that outcome even if the original instruction was accepted.

The wire model comes from `s2-kit`. The interoperability check exercises a pinned, authenticated WebSocket session with the upstream S2 Analyzer, including a live battery fill reading and the outcome for an undriven asset. [S2 Analyzer](https://github.com/flexiblepower/s2-analyzer) · [Control authority](@/docs/architecture.md#the-arbiter).
