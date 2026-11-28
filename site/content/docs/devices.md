+++
title = "Devices and drivers"
description = "Commission industrial OPC UA meters and household SunSpec, Modbus and EEBUS devices. Diagnose missing readings and inspect meter evidence."
weight = 11

[extra]
group = "Control and metering"
+++

Choose a driver for the measurements and commands the installed device actually supports. The site configuration identifies the asset and its role. Discovery identifies the device at the other end of the connection. A responding TCP endpoint alone cannot verify an inverter or a connection meter.

| Household integration | What to verify |
|---|---|
| SunSpec over Modbus TCP | Unit, model list, role, rating and the sign and direction of measured power |
| Wired M-Bus heat meter | Dedicated level converter, primary address, manufacturer and eight-digit meter ID |
| SunSpec over Modbus RTU | Named RS-485 bus, unit address, model 203 and meter serial number |
| Vendor Modbus map | Register space and address, width, word order, scale and field from the device’s register manual |
| EEBUS Steuerbox | Peer identity, accepted reduction and time since the last heartbeat |
| EEBUS appliance | Supported use case, whether it measures or commands, and the commissioned peer |
| Matter appliance | Commissioned endpoint and fabric in the local Matter Server. The pilot reads active power. [Setup and reading states](@/docs/household.md#matter-appliance-power). |

The household OCPP simulator uses an outlet energy register to check one charging transaction across reconnect. Run the [departure rehearsal](@/docs/simulation.md#check-an-ev-departure).

## Read a wired heat meter

Connect one wired M-Bus heat meter through a dedicated M-Bus level converter. Copy its primary address (0–250), three-letter manufacturer code and eight-digit ID into the commissioned household configuration. Keep leading zeros inside the quotes. The converter uses 8 data bits, even parity and one stop bit.

```toml
[heat_meter]
path = "/dev/serial/by-id/usb-mbus-converter"
address = 17
manufacturer = "KAM"
meter_id = "06855817"
baud = 2400
poll_s = 60
```

You can also provision this table before first-run Setup. Commissioning the electrical connection meter preserves the heat-meter settings.

The defaults are 2400 baud and 60 seconds between completed reads. 9600 baud and poll intervals of 2–3600 seconds are also supported. Choose an interval allowed by the meter manufacturer. Validate the configuration with `hemsd run --config house.toml --check`, restart the gateway and open **Setup → Heat meter**. Compare the cumulative thermal Wh with the meter display, accounting for kWh or MWh on the dial.

The profile reads one plaintext long-header reply from a heat inlet or outlet meter. It requires a clear device status and one unmodified current Wh-family total for the whole meter. A reply can also contain historical, tariff or subunit registers. The gateway distinguishes those from the total. Multiple telegrams, combined heating/cooling meters, joule registers and wireless receivers require other profiles.

Setup shows the gateway receipt time of the last accepted reading. A reply from a different primary address or printed meter ID shows **Unexpected meter**. A telegram that asks for a second page shows **Unusable reading**. A silent converter shows **Reading unavailable**. Each fault clears the current value and keeps the last successful read time. Check the wiring and commissioned identity, then wait for the next poll. The same result is available in `hemsd doctor` and the authenticated `GET /v1/trust` response under `heat_meter`. Thermal energy stays outside the electrical balance. Run `just heat-meter-trial` on macOS or Linux with Chrome or Chromium to rehearse meter substitution, loss and recovery. [mbus-kit](https://github.com/hupe1980/mbus-kit)

## Commission a serial grid meter

Use this path for a three-phase wye meter that exposes SunSpec model 203 over Modbus RTU. Give each meter on the RS-485 line a unique unit address (1–247). Set the same baud rate, parity and stop bits on every device. Use an adapter with automatic transmit direction or a UART whose system driver handles RS-485 direction. The [WattNode WND-WR-MB reference manual](https://ctlsys.com/wp-content/uploads/2023/07/WND-WR-MB-WattNode-Ref-Manual.pdf) describes one meter with this profile.

1. Create a new `house.toml` containing only the bus declaration below. Replace the example path with your adapter’s local device path. On Linux, a persistent path under `/dev/serial/by-id/` survives device renumbering.
2. Start `hemsd run --config house.toml`, sign in with the printed code and choose the RS-485 bus in Setup. Enter the grid meter’s unit. Compare the reported manufacturer, model and serial number with the meter label before confirming.
3. After restart, open **Setup → Serial meters**. Check signed active power against the meter display and check the import and export counters. If a meter stops responding, its last reading time remains visible and its values disappear until a new reading arrives.

```toml
[serial_buses.cabinet]
path = "/dev/serial/by-id/usb-grid-meter-adapter"
baud = 19200
parity = "even"
stop_bits = 1
```

For a separately metered base load, add a second `[[drivers]]` entry to the commissioned configuration with `kind = "serial-sunspec"`, `asset = "haushalt"`, the same `bus` and its own `unit`. The gateway polls both meters through one port. Its local API exposes the readings in `GET /v1/trust`.

On macOS or Linux, run `just serial-meter-trial` with Chrome or Chromium installed to rehearse commissioning, a meter replacement and a lost reading over a pseudo-terminal.

## What a device will actually take

The arbiter requests watts. An inverter accepts a production ceiling, an EV charger accepts current per conductor and a fixed-speed appliance has a small set of operating states. The device model translates a permitted range into a physically attainable command before the driver encodes it.

For example, a three-phase charger with a minimum current of 6 A per phase needs about 4.1 kW at 230 V. If only 3 kW remains under a shared limit, requesting the minimum on all three phases would exceed the limit. Charging pauses unless a supported phase configuration allows the lower power. SG Ready contacts also need an explicit mapping. Different states express operating recommendations and restrictions, and do not form a linear watt scale.

## The driver contract

A driver declares what it measures and what it can command. A site may assign one device separate measurement and command paths when their responsibilities do not overlap. The registry rejects conflicting ownership. `ems-drv` parses messages and emits events. The gateway schedules reads, manages sockets and applies the site’s limits.

Polled devices need timely responses. A subscription that publishes only changes can keep the same temperature for an extended period. The registry uses the freshness semantics of the driver and its connection state. Setup distinguishes a quiet or disconnected device from a device that replied to a command but failed verification.

## SunSpec and vendor registers

The gateway walks the SunSpec model chain on the configured unit. Common supported roles include inverter models 101–103 or 701, meter models 201–204 and battery model 802. Verify the discovered operating model before applying the configuration in Setup. The virtual battery reports AC power on model 701 and state of charge on model 802. It runs on self-consumption by default. Start the virtual house with `--manage-storage` to let the gateway request signed watts on model 704. Use a control tick of 1–3 seconds for this profile. The battery returns to self-consumption when its 30-second control period expires.

Set `listens_only = true` for a read-only SunSpec unit. On reconnect the driver rediscovers the unit before accepting a ceiling. For supported inverter controls, a Modbus write acknowledgement confirms receipt of a request. The driver reads back the active limit and enable state. Compare the resulting command outcome with measured production and available sunlight. A roof producing below a ceiling on a cloudy day gives no evidence that curtailment is active.

Vendor register maps are read-only by default. Each permitted write must declare its register, command and allowed raw values. A heat pump with a documented heat/cool mode register can therefore support a mode change without permitting arbitrary writes. Use the exact register table for the installed firmware.

For a fixed three-phase wallbox, `charging_amps` writes a whole-ampere limit per conductor. The gateway rounds down and checks the echoed register address and count. A planner session needs fresh cable presence, charge fraction, usable capacity and a household departure goal. The virtual house generates a working register map and goal with `--with-ev`. [Run the simulation](@/docs/simulation.md).


## `eebus` — the § 14a side

The house accepts an EEBUS SHIP connection from its paired Steuerbox. The gateway records the reduction, acknowledgement and heartbeat and passes the limit to the guard. For appliances, the configured household gateway can connect to the appliance’s SHIP endpoint. Depending on its use cases, it can read EV connection and state of charge, monitor a tank temperature or request a compressor operating state. [Grid limitation state machine](@/docs/grid-rules.md#the-eebus-limitation-machine).

## Commission and diagnose

Validate the configuration before starting the daemon. A configuration check catches inconsistent site declarations. A live check also needs the gateway credential and a reachable device.

```sh
hemsd run --check --config house.toml
hemsd doctor --config house.toml
```

In Setup, select the intended role, review the discovered identity and readings, and check the proposed configuration before applying. Inspect telemetry and command outcomes in the [local API](@/docs/api.md). For a bench run with a virtual inverter, meter, battery and control box, follow the [virtual-house quickstart](@/docs/getting-started.md#a-house-without-hardware).

## Commission an industrial OPC UA meter

Use `cemsd capture-meter` with a reviewed import register and the server's pinned certificate. Each observation retains its source timestamp, status and original numeric precision. Import direction and reset period come from the site's point list.

Open the capture with `cemsd commission-meter --format html` and supply the site's independent quarter-hour closes. **Go to first exception** takes you to the affected Berlin quarter. Under **First excluded reading**, look up the capture line and reason. When the meter supplies a source time, follow the link to the corresponding quarter. A shared boundary links to both quarters. Check the UTC offset during the repeated hour. A missing boundary leaves the difference unknown. If Float precision is the limiting factor, a cumulative Float reaches its 1 Wh resolution limit at 16,777,216 Wh. [Command examples and sample capture](https://github.com/hupe1980/ems/tree/main/services/cemsd#commission-an-opc-ua-import-meter).

<a href="../../screens/industrial-meter-review.png"><img src="../../screens/industrial-meter-review.png" width="650" style="max-width:100%;height:auto" loading="lazy" alt="Commissioning report identifying an excluded register reading by capture line, OPC UA status and source time, with links to the two adjoining quarters"></a>
