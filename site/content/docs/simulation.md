+++
title = "Simulation and meter replay"
description = "Test a household gateway with virtual devices, replay OCPP charging limits and community shares, and check an industrial dispatch against its meter and OpenADR request."
weight = 12

[extra]
group = "Integration and operations"
+++

Choose the boundary you want to exercise. All peers below run on the same machine.

| Run from the workspace root | What crosses the connection | What the simulation supplies |
|---|---|---|
| `cargo test -p hemsd --features house --test virtual_house` | The household gateway reads SunSpec registers over Modbus TCP and receives grid limits over EEBUS. | Virtual equipment, grid signals and household demand |
| `just matter-recovery` | The gateway reads appliance power through a schema-11-compatible Matter Server WebSocket API. | The commissioner peer, its readings and a connection loss. The test ends at the commissioner API. |
| `cargo run -p bemsd -- demo` | Two charging stations exchange OCPP 1.6J messages with a local charging management system. | Station loads, meter timestamps and delivery delays |
| `just opcua-meter-trial` | An independent simulated OPC UA server supplies a source-timed import register over an encrypted session. | Register identity, capture quality and exact Wh against independent closes |
| `cargo run -p cemsd --features openadr -- openadr` | A site client exchanges an event and a metered report with an OpenADR 3.1 server over HTTP. | Aggregator request, industrial meter trace and alternative battery dispatch times |

## Check a winter hot-water target

From the workspace root, save the receipt and open it in a browser. Omit `--html` for JSON.

```sh
cargo run --locked -p hemsd --features house -- thermal-trial --target-wh 2300 --html > winter.html
```

Two simulated tanks start in the same state. S2 requests full power from one and half power from the other. Independent appliance readings show 2,380 Wh and 2,005 Wh of usable heat at the deadline. Only the first meets the 2,300 Wh goal. The SunSpec meter measures whole-house import.

Use `--missing-close` to inspect an incomplete receipt, `--disconnect` to let an instruction expire, or `--departure-wh 3600` to follow the same meter through an OCPP departure at 07:00 UTC.

## Read a winter day with storage

`cargo run --locked -p hemsd --features house -- winter-day` prints the Berlin winter day from midnight to midnight. Open **Day → Winter trial** in the household demo built with the `house` feature, or use `--html` to save the report and inspect any quarter offline. Use `--json` for the 97 cumulative meter readings and the battery’s opening and closing state of charge. Each arm runs the same rooftop production, household demand and self-managed battery. Only the morning heater request differs. Use `--target-wh` and `--departure-wh` to change the service goals.

The default run imports 6,592 Wh with the full heater request and 6,467 Wh with the half request. Both batteries finish at 9.77% after standby losses. The lower-import run misses the 06:15 UTC hot-water target by 295 Wh. Room and water observations cover that morning quarter. The electrical day runs in accelerated time.

<a href="../../screens/winter-meter-day.png"><img src="../../screens/winter-meter-day.png" width="600" style="max-width:100%;height:auto" loading="lazy" alt="Winter receipt comparing meter totals, battery charge and the baseline hot-water shortfall"></a>

## Reconcile the virtual household meter

Run `just house-meter-trial` from the workspace root. It reads import and export over Modbus TCP for three Berlin dates, advancing the plant one second at a time. Winter has 96 quarters, the spring clock change has 92 and the autumn change has 100. The closing midnight reading belongs to the last quarter of the preceding day. Each quarter must agree with integrated device power within 1 Wh per direction. It also compares the daily totals with a separately stepped minute model. The allowance in each direction is the larger of 50 Wh and 1% of that model’s total. The printed fixture bill uses 25 ct/kWh for import before 18:00 Berlin time, 40 ct/kWh thereafter, and 8 ct/kWh for export. It must agree with the plant calculation within one cent.

For a live session run `cargo run -p hemsd --features house -- house --run --at 16:58`. The gateway manages the inverter and reads the connection meter and the self-managed battery. The virtual Steuerbox sends its scheduled reduction over EEBUS. The `grid` entry in `/readyz` shows whether the paired Steuerbox is connected. Setup shows the last LPC heartbeat. Keep the owner-only `.steuerbox.pem` beside the generated configuration to preserve the paired identity across restarts.

### Measure space and water heating

Add either flag to `house --run`, or combine them. Both appliances run on loopback. The gateway reads temperature (0.1 °C) at holding register 0 and measured electrical power (W) at register 1. Their actual draw appears at the same connection meter.

| Flag | Modbus unit | Control at holding register 2 |
|---|---|---|
| `--with-tank` | 4 · hot-water cylinder | 0 switches the relay off, 1 enables the heater. Draws and standing losses cool the tank. |
| `--with-room` | 5 · space heating | 1 limits the compressor to zero, 2 uses the normal thermostat, 3 raises its target to the top of the comfort band. |

### Verify a vehicle departure

Run `hemsd house --day winter --at 03:00 --with-ev --run` to commission a three-phase wallbox over loopback Modbus TCP. The winter car starts at 18 kWh and needs 38 kWh at 07:00 Berlin simulation time. Open the household view to pause or boost charging. The connection meter includes the rest of the house. Add `--with-room --with-tank --manage-storage` to share the meter with heating and storage. Run `just house-session-trial` to compare a 01:30 and a 02:00 start at 6 A per phase through the 07:00 departure. The outlet measures AC energy. The car reports its stored energy at departure.

| Charging starts | Outlet delivery | Car at 07:00 | 38 kWh target |
|---|---:|---:|---|
| 01:30 | 22.770 kWh | 38.946 kWh | Met |
| 02:00 | 20.700 kWh | 37.044 kWh | Short by 0.956 kWh |

To compare a running gateway with a fresh reference planner, start the house in an empty working directory. In a second terminal there, run the planner after the gateway writes its configuration.

```sh
hemsd house --run --day winter --with-room --with-tank --with-ev --manage-storage --keep-history
```

```sh
hemsd simulate --day winter --cold-start --site-from house.toml --json > reference.json
```

The gateway retains its quarters and closed-day reports in `house.toml.redb`. The reference uses the same installed ratings with its own clock, weather and prices. Inspect `battery_soc_end` and `tank_closing_kwh` alongside service at the deadlines before comparing cost.

## Check an EV departure

Run one household wallbox and its controller over local OCPP 1.6J WebSockets. The car plugs in at 06:15 UTC, charging stops at 06:30, and the station reconnects at 06:45. Departure is at 07:00. The 45 simulated minutes run in seconds.

```sh
cargo run -p hemsd --features house -- ev-departure
```

| Try a goal | What the outlet register proves |
|---|---|
| Default, 3,600 Wh | 1,200 Wh by rejoin and 3,600 Wh by departure. The goal is met. |
| `--goal-wh 6000` | 4,750 Wh delivered. The 11 kW wallbox can add at most 2,750 Wh in the last quarter. The shortfall is 1,250 Wh. |
| `--missing-close` | 1,200 Wh by rejoin. The departure total stays unknown. |

The **Connection meter** line covers the whole 45-minute window. With the default goal, the house imports **4,281 Wh**, including **230 Wh** while charging is paused. The outlet accounts for **3,600 Wh**. With `--missing-close`, the same grid total is available but the last verified outlet delivery is only 1,200 Wh. Roof production and household loads run on the same clock as the outlet, with storage disabled.

The household guard reads connection power over Modbus TCP before each charging decision. The wallbox applies a 15-minute transaction profile, and its cumulative Wh register survives the interruption. The goal is measured at the outlet. Vehicle state of charge and charging losses are outside this fixture.

## Exercise battery control

Run `cargo run -p hemsd --features house -- house --run --manage-storage` to let the gateway control the simulated pack. Open the battery’s control detail in **Now** to compare requested watts, the setting confirmed by the device, and measured AC power. The detail retains the last confirmation time when feedback goes quiet. Omitting the flag leaves the pack in self-consumption.

`just house-storage-trial` exercises the gateway driver and an independent controller over Modbus TCP. Unit 3 reports AC power in model 701, state of charge in 802 and the active setting in 704. Negative device watts charge the pack. The gateway renews a target on a fresh control decision. The peer releases it after 30 simulated seconds without renewal or on explicit disable. [SunSpec DER AC controls](https://github.com/sunspec/models/blob/master/json/model_704.json)

A separate controller can discover model 704 on unit 3 and send a four-register write starting at `WSetEna`. Set enable and watt mode to 1, then supply the signed 32-bit `WSet` value, high word first. Read back the setting and `WSetRvrtRem`. Writing 0 to `WSetEna` releases the pack immediately.

## Check a shift against its meter

From the workspace root, run:
```sh
cargo run -p cemsd -- reconcile crates/ems-sim/fixtures/shift-trace.json
cargo run -p cemsd -- reconcile --format html crates/ems-sim/fixtures/shift-trace.json > review.html
```

The command compares cumulative import with closed quarter-hour energy for one
delivery point. Supply whole Wh, explicit UTC offsets and a tolerance in Wh.
Each closed reading uses the interval start. Export the latest corrected
values before running. The example crosses the repeated autumn clock hour.
Its two differences are 0 Wh and +1 Wh (register minus closed meter).

Open `review.html` offline. **Go to first exception** jumps to the affected quarter. **Next exception** skips matching intervals. Each quarter shows both readings, its signed difference and its Berlin offset.

The terminal summary gives coverage and the largest difference. Redirect stdout to save the JSON report. A missing boundary, decreasing register, missing
closed interval or excessive difference yields a nonzero exit with the interval
report. Conflicting duplicates and malformed input yield a diagnostic on stderr.
The local file limit is 4 MiB and the comparison window is at most 31 days.

## Replay a building handover

Run `cargo run -p bemsd -- demo` and open
[the simulated Westgebäude](http://127.0.0.1:8081/). The German
handover shows grid headroom, PV, shared loads and the last charging reduction.

Open **Nachweis je Ladestation** for the two simulated OCPP 1.6J stations. In `?case=stale-charger`, both stations accepted the profile, but West-02 has no aligned station reading. The bank total is unknown. In `?case=idle-station`, West-02 reports 0 kW. Both readings count and the bank total can be calculated.

Each station initially draws 14 kW and receives a 12 kW limit. In `?case=above-request`, each draws 1 kW above its limit after ramp-down. The bank therefore draws 26 kW at 09:12:15 against the 24 kW request. Its 24 kW snapshot at 09:15 belongs to a later observation.

Open [the delayed reading at its deadline](http://127.0.0.1:8081/?case=delayed-arrival&moment=deadline). At 09:12:15 the operator still lacks one station’s reading. Advance to handover to see when it arrived and what it confirms. Step back to inspect the original uncertainty.

Choose *Bestätigung fehlt* to replay the same request with its final meter
sample missing. Reloading preserves the selected case. The demo uses synthetic
data and exposes these read-only routes on its loopback listener.

### When a charger reconnects

In **Beispielfall wechseln**, choose **West-02 nach Verbindungsabbruch**. At the deadline West-01 reports 12 kW and West-02 is disconnected. Follow **Weiter**. West-02 reconnects and resends a pre-loss reading. The bank total stays unknown until the 09:13:30 MEZ samples arrive. The two stations then report 12 and 14 kW, exceeding the original request by 2 kW. **Zurück** restores the evidence available at the selected instant.

### Check the heating reserve

With the building demo running, open [Heizreserve](http://127.0.0.1:8081/comfort). Analog Input 1 reports 21.5 °C over BACnet/IP. The two OCPP stations can share 24 kW while the point is healthy. A fault at the same temperature retains 6 kW for heating. When the point goes silent, the last good reading remains visible with its age. Compare the requested ceiling with the independently received station readings in each case. The 24 kW budget and 6 kW reserve are inputs to this fixture.

In the sensor fault case, open **KNX-Zähler am Ladeabgang vergleichen**. A separate virtual feeder meter sends signed watts on group `2/1/3` (DPT 14.056) over loopback UDP. The three alternatives use the same 18 kW station total. A 22 kW feeder reading shows a 4 kW excess. A telegram received three seconds before the station deadline is too old to compare. The rehearsal allows 100 W of difference and up to two seconds of receipt age.

## Compare community allocations

For community energy sharing, run `cargo run -p bemsd -- community --viewer coordinator`, open [the allocation replay](http://127.0.0.1:8083/) and use the login printed in the terminal. Compare the two keys on 10 kWh of supply. The static key leaves 3 kWh undistributed. The cascading key assigns those 3 kWh to the member with unmet demand. Switch to the second 02:00 interval and check its UTC offset. With a missing member reading the coordinator sees incomplete coverage and the final allocation is withheld. Restart with `--viewer member-a` and the new login to see the member’s own share.

In the coordinator view select **Welche Messung entscheidet?**. Reveal A or B with the keyboard. The page shows what that single reading establishes and withholds final shares until both meters are measured. Back and reload restore the selected evidence.

## Replay an industrial shift

Try the [peak challenge](@/peak-challenge.md) for a five-minute decision in your browser. Both choices use the same meter trace as the local demo. The original close records the quarter with the battery idle. The local demo adds stale meter readings, uncertain battery capacity and the repeated clock hour.

Run `cargo run -p cemsd -- demo` and open [the industrial shift replay](http://127.0.0.1:8082/). Dispatch at 02:55 UTC+2 and the 120 kW battery can hold the interval mean to 1,100 kW. Wait until 02:57 UTC+2 and the same battery leaves 1,116 kW. Reveal the unmodified meter trace to see the original 1,140 kW close at 02:00 UTC+1. The battery alternative uses a synthetic fixed annual
demand rate. Read the matching figures at `/v1/demo/industrial?moment=delayed`.

## Replay an aggregator request

Run the optional OpenADR 3.1 simulation from the workspace root:

```sh
cargo run --locked -p cemsd --features openadr -- openadr
cargo run --locked -p cemsd --features openadr -- openadr --cancel-before-dispatch --json
```

The first command starts a temporary aggregator and site client on loopback. The aggregator requests a **1,100 kW mean over one quarter hour**. This fixture uses the private OpenADR payload `EMS_QUARTER_MEAN_LIMIT` with units `KW` for the target. An instantaneous import ceiling would describe a different constraint.

Compare the site’s two projected means with the original meter trace. Starting early projects 1,100 kW and waiting projects 1,116 kW. The recorded interval contains 285 kWh, or 1,140 kW on average. Only the recorded energy belongs in the usage report. The second command withdraws the event before dispatch. Add `--json` to either run for a machine-readable receipt.

## Winter report

Add `--cold-start` to rehearse the first day after installation. The report shows the source of the initial load estimate and how much of the day relies on it.

`cargo run -p hemsd -- simulate --day winter` prints the following readings. Net savings include battery wear and comfort costs.

```text
  2026-01-15 — with a § 14a reduction

  produced                                  7.4 kWh
  household consumption                    11.0 kWh
  charged into the car                     21.7 kWh
  heat pump                                22.1 kWh
  battery throughput                       14.6 kWh
  imported                                 52.2 kWh
  self-sufficiency                             12 %

  electricity bill                          20.50 €
  battery life spent                         0.58 €
  comfort given up                           0.13 €
  cost of the day                           21.61 €
  without optimisation                      22.68 €
  saved                                      1.07 €
  …of it on the bill                         0.80 €

  § 14a limit in force                       90 min
  …against a minimum of                     10.5 kW
  …covered by the store                     1.4 kWh
  § 14a limit respected                         yes
  § 9 EEG ceiling respected                     yes

  production forecast, CRPS  173 W (83 % of 30 lit)
  load forecast, CRPS           18 W (84 % covered)
```

## The reference household

Every reference day is the same house unless the day says otherwise: a common
German single-family home in 2026.

| | |
|---|---|
| Roof | 9,8 kWp of modules on an 8 kW inverter |
| Battery | 10 kWh, 5 kW both ways, 10 % kept back for a power cut, wear priced at 8 ct/kWh of throughput |
| Charge point | 11 kW, able to drop to one conductor; the household's own Ladelimit at 75 % |
| Heat pump | 5 kW electrical, modulating, comfort band 20–23 °C |
| Hot water | 300 litres on a 0,5 kW hot-water heat pump |
| Dishwasher | a ninety-minute programme: heat, wash, heat to dry |
| Connection | 35 A main fuse |
| § 9 EEG | an intelligent metering system since 2024, **and the 60 % cap still on** |


The reference installation has a meter and a separate controllability-test status. Use the scenario switch `--imsys` to compare the case where the § 9 cap is lifted.

## Seven days

Run `just demo-all` for the reference scenarios. To inspect one, use `cargo run -p hemsd -- simulate --day NAME`.

| Scenario | What to inspect |
|---|---|
| `winter` | The 17:00 reduction, overnight car target and cost of the day |
| `summer` | Rooftop surplus and three hours of negative prices |
| `deadline` | A car arrives as a reduction starts, with three hours to charge |
| `shared` | A household without storage shares a limit that arrives between planning boundaries |
| `offline` | Local fallback when the planner is switched off |
| `autumn` | A charger operating where the available surplus can support one phase |
| `capped` | Curtailment on a 20 kWp roof under a binding export cap |

Use `--perfect-foresight` to isolate forecast errors on the same weather, `--wear-eur-per-kwh 0` to isolate battery wear, and `--no-phase-switching` to compare charging on fixed wiring. A savings comparison needs the same site, service expectations and applicable grid limits in both runs.

## What the simulations exercise

The charger can refuse a request below its minimum current. A battery reports delivered energy after power-dependent losses, and an inverter takes time to reach its ceiling. The virtual control box can stop sending heartbeats during a reduction. Inspect the meter or command outcome to distinguish what was requested from what happened. [Device behaviour](@/docs/devices.md).

For a socket-level trial, start the [virtual house](@/docs/getting-started.md#a-house-without-hardware). The paired electrical trials use seven weather and price profiles with fixed load and rooftop PV. A shadow gateway observes meter and device readings while an independent controller remains responsible for limits. A separate mixed-asset trial runs two physical models with the same charging window, heating demand, hot-water draw and initial storage. It reads the simulated connection meters via SunSpec/Modbus TCP. The test includes an equal-bill control, a cheaper day that misses departure and a lost service sample.

## One day cannot answer some questions

```sh
just backtest summer 20
just risk deadline 20
```

The first command varies the seeded weather and evaluates forecast bands over twenty episodes. The second compares planning policies across repeated deadline scenarios. Inspect the worst outcome and unmet service alongside mean cost. An ordinary day with no missed deadline says little about performance in a tight evening.

In the household demo, Day and Month compare electricity bills only after both minute-by-minute trajectories qualify. Both rooms must stay within 0.1 K of the comfort band. Departure energy, daily unmet hot water and final stored energy are checked to 1 Wh. Different grid-control regimes withhold the comparison. The CLI also reports its separate economic benchmark, including wear and discomfort.

Reference days are reproducible model comparisons. A wall-clock solver budget can affect the solution selected on a busy machine. A field savings claim requires metered costs and comparable service at the installation. [Forecast quality](@/docs/forecasting.md) · [How receipts use measured data](@/docs/household.md#the-month).
