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
| `cargo run -p cemsd --features openadr -- openadr` | A site client exchanges an event and a metered report with an OpenADR 3.1 server over HTTP. | Aggregator request, industrial meter trace and alternative battery dispatch times |

## Check a shift against its meter

From the workspace root, run:
```sh
cargo run -p cemsd -- reconcile crates/ems-sim/fixtures/shift-trace.json
```

The command compares cumulative import with closed quarter-hour energy for one
delivery point. Supply whole Wh, explicit UTC offsets and a tolerance in Wh.
Each closed reading uses the interval start. Export the latest corrected
values before running. The example crosses the repeated autumn clock hour.
Its two differences are 0 Wh and +1 Wh (register minus closed meter).

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

## Compare community allocations

For community energy sharing, run `cargo run -p bemsd -- community --viewer coordinator`, open [the allocation replay](http://127.0.0.1:8083/) and use the login printed in the terminal. Compare the two keys on 10 kWh of supply. The static key leaves 3 kWh undistributed. The cascading key assigns those 3 kWh to the member with unmet demand. Switch to the second 02:00 interval and check its UTC offset. With a missing member reading the coordinator sees incomplete coverage and the final allocation is withheld. Restart with `--viewer member-a` and the new login to see the member’s own share.

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
  without optimisation                      22.70 €
  saved                                      1.09 €
  …of it on the bill                         0.83 €

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

For a socket-level trial, start the [virtual house](@/docs/getting-started.md#a-house-without-hardware). The paired electrical trials use seven weather and price profiles with fixed load and rooftop PV. A shadow gateway observes meter and device readings while an independent controller remains responsible for limits. Comparisons involving a connected car, hot-water draw or shifted battery inventory require evidence that both alternatives delivered equal service.

## One day cannot answer some questions

```sh
just backtest summer 20
just risk deadline 20
```

The first command varies the seeded weather and evaluates forecast bands over twenty episodes. The second compares planning policies across repeated deadline scenarios. Inspect the worst outcome and unmet service alongside mean cost. An ordinary day with no missed deadline says little about performance in a tight evening.

Reference days are reproducible model comparisons. A wall-clock solver budget can affect the solution selected on a busy machine. A field savings claim requires metered costs and comparable service at the installation. [Forecast quality](@/docs/forecasting.md) · [How receipts use measured data](@/docs/household.md#the-month).
