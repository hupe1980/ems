+++
title = "Devices and drivers"
description = "Verify SunSpec, Modbus and EEBUS devices, follow commands and diagnose missing readings."
weight = 11

[extra]
group = "Control and metering"
+++

Choose a driver for the measurements and commands the installed device actually supports. The site configuration identifies the asset and its role. Discovery identifies the device at the other end of the connection. A responding TCP endpoint alone cannot verify an inverter or a connection meter.

| Household integration | What to verify |
|---|---|
| SunSpec over Modbus TCP | Unit, model list, role, rating and the sign and direction of measured power |
| Vendor Modbus map | Register space and address, width, word order, scale and field from the device’s register manual |
| EEBUS Steuerbox | Peer identity, accepted reduction and time since the last heartbeat |
| EEBUS appliance | Supported use case, whether it measures or commands, and the commissioned peer |
| Matter appliance | Commissioned endpoint and fabric in the local Matter Server. The pilot reads active power. [Setup and reading states](@/docs/household.md#matter-appliance-power). |

## What a device will actually take

The arbiter requests watts. An inverter accepts a production ceiling, an EV charger accepts current per conductor and a fixed-speed appliance has a small set of operating states. The device model translates a permitted range into a physically attainable command before the driver encodes it.

For example, a three-phase charger with a minimum current of 6 A per phase needs about 4.1 kW at 230 V. If only 3 kW remains under a shared limit, requesting the minimum on all three phases would exceed the limit. Charging pauses unless a supported phase configuration allows the lower power. SG Ready contacts also need an explicit mapping. Different states express operating recommendations and restrictions, and do not form a linear watt scale.

## The driver contract

A driver declares what it measures and what it can command. A site may assign one device separate measurement and command paths when their responsibilities do not overlap. The registry rejects conflicting ownership. `ems-drv` parses messages and emits events. The gateway schedules reads, manages sockets and applies the site’s limits.

Polled devices need timely responses. A subscription that publishes only changes can keep the same temperature for an extended period. The registry uses the freshness semantics of the driver and its connection state. Setup distinguishes a quiet or disconnected device from a device that replied to a command but failed verification.

## SunSpec and vendor registers

The gateway walks the SunSpec model chain on the configured unit. Common supported roles include inverter models 101–103 or 701, meter models 201–204 and battery model 802. Verify the discovered operating model before applying the configuration in Setup. The virtual battery exposes state of charge and manages its own power.

For supported inverter controls, a Modbus write acknowledgement confirms receipt of a request. The driver reads back the active limit and enable state. Compare the resulting command outcome with measured production and available sunlight. A roof producing below a ceiling on a cloudy day gives no evidence that curtailment is active.

Vendor register maps are read-only by default. Each permitted write must declare its register, command and allowed raw values. A heat pump with a documented heat/cool mode register can therefore support a mode change without permitting arbitrary writes. Use the exact register table for the installed firmware.

## `eebus` — the § 14a side

The house accepts an EEBUS SHIP connection from its paired Steuerbox. The gateway records the reduction, acknowledgement and heartbeat and passes the limit to the guard. For appliances, the configured household gateway can connect to the appliance’s SHIP endpoint. Depending on its use cases, it can read EV connection and state of charge, monitor a tank temperature or request a compressor operating state. [Grid limitation state machine](@/docs/grid-rules.md#the-eebus-limitation-machine).

## Commission and diagnose

Validate the configuration before starting the daemon. A configuration check catches inconsistent site declarations. A live check also needs the gateway credential and a reachable device.

```sh
hemsd run --check --config house.toml
hemsd doctor --config house.toml
```

In Setup, select the intended role, review the discovered identity and readings, and check the proposed configuration before applying. Inspect telemetry and command outcomes in the [local API](@/docs/api.md). For a bench run with a virtual inverter, meter, battery and control box, follow the [virtual-house quickstart](@/docs/getting-started.md#a-house-without-hardware).
