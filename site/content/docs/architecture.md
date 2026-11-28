+++
title = "Edge architecture"
description = "How the guard, arbiter and planner share control on the gateway and work offline."
weight = 4

[extra]
group = "Control and metering"
+++

The project has three edge applications. `hemsd` operates a household gateway. `bemsd` replays a building operator’s handover and community allocations. `cemsd` captures a commissioned OPC UA import register, reconciles it with independent closes and replays industrial peak decisions. The building and industrial demos use synthetic measurements. [Run the demos](@/docs/getting-started.md).

## Three planes

The household has to react to a grid instruction within a control cycle even while it is calculating the cheapest time to charge a car. Three components work at different timescales.

| Component | Input | Output |
|---|---|---|
| Guard | Grid instructions, site wiring, device ratings and current measurements | Permitted power range for each asset and a reason for each restriction |
| Arbiter | Permitted ranges, household requests, manager instructions and planned energy | A device command each control tick, typically once a second |
| Planner | Effective prices, forecasts, available stores and household goals | Quarter-hour energy targets, flexibility ranges and marginal values |

### The guard

#### Two kinds of limit, and the difference matters

A battery reserve or inverter rating limits one device. A connection fuse or a network operator’s reduction can constrain several devices together. If two chargers each receive the entire 6 kW remaining at a connection, they can draw 12 kW. The guard allocates shared headroom before the arbiter issues individual commands.

The guard uses fresh measurements to establish headroom. It excludes assumed loads, loads it is stopping and demand an appliance can withdraw independently when those quantities would overstate safe export. A device’s commanded output and its confirmed output are separate observations. [Grid constraints and measurement evidence](@/docs/grid-rules.md).

### The arbiter

Within the guard’s bounds, household overrides have priority over an external manager’s instruction, followed by the current plan and the local fallback. A device must also be able to realise the request. For example, cutting a three-phase EV charger below its minimum charging current can require a pause.

The arbiter tracks energy delivered over the slot. If an appliance briefly draws less than requested, the remaining target reflects the actual delivery. A stale plan expires. During fallback the gateway can continue local self-consumption control and enforce grid limits. [Device commands](@/docs/devices.md).

### The planner

The default horizon is 192 quarter-hours, or 48 hours. The gateway acts on the current slot and replans as measurements or household goals change. A battery needs a fresh state of charge to participate. A car needs a connected vehicle and sufficient state for its goal. An omitted device receives no target from that plan. [Planner inputs and costs](@/docs/optimizer.md).

## Measurements and network connections

Power follows the load convention throughout the control loop. Positive grid power means import. An inverter producing 3 kW reports −3 kW, so a 4 kW load served by that roof gives +1 kW at the connection. [Domain model](@/docs/domain-model.md#one-sign-convention).

The gateway owns sockets, time and persistence. Domain crates take measurements, events and time as inputs. A simulation can therefore run the same guard and arbiter against a recorded sequence. Protocol drivers consume bytes and produce typed measurements or outbound frames. The gateway handles reconnects and schedules polling.

For network-assisted planning, the box requests prices from `tariffd` and weather from `forecastd`. It models its own roof and retains learned corrections locally. Loss of the network can reduce the quality of a plan while the local control loop continues to enforce its limits. External managers connect through a separately authorized S2 session. [Forecasting](@/docs/forecasting.md) · [Fleet services](@/docs/services.md) · [S2 flexibility](@/docs/flexibility.md).
