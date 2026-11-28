+++
title = "Energy units and site model"
description = "Read power signs, quarter-hour intervals, electrical topology and device limits."
weight = 5

[extra]
group = "Control and metering"
+++

`ems-core` is the vocabulary every other crate speaks. It holds **facts and no
rules**: what a site is made of, what a quarter hour is, what a measurement and
a command are. Whether a heat pump is a *steuerbare Verbrauchseinrichtung* under
§ 14a is a rule and lives in [`ems-grid`](@/docs/grid-rules.md); what a
kilowatt-hour costs is [`ems-tariff`](@/docs/tariffs.md); what to do about
either is [`ems-optimizer`](@/docs/optimizer.md) and `ems-realtime`.

The regulatory vocabulary the workspace shares — `Fallgruppe`, the Bundesland
calendar, the market identifiers — comes from
[`metering`](https://github.com/hupe1980/metering), so hems and the metering
layer can never disagree about what a quarter hour or a Fallgruppe is.

## One sign convention

Every power and energy value uses the **load convention**: positive is power
flowing *into* the thing being measured.

| Thing | Positive | Negative |
|---|---|---|
| Grid connection | import (Netzbezug) | export (Einspeisung) |
| Photovoltaic array | standby draw at night | production |
| Battery | charging | discharging |
| Wallbox | charging the car | discharging it (V2H / V2G) |
| Heat pump, hot-water tank, household load | consumption | — |

`Site::balance_residual` compares the connection reading with the sum of asset powers. A persistent discrepancy can indicate an unmetered load or a reversed sign.

```text
grid connection power = sum of asset powers behind the connection
```

## The quarter-hour grid, and the two days a year that are not 96 slots

A `Slot` identifies a quarter-hour by its UTC start. Convert to Europe/Berlin when grouping receipts by local date or applying a wall-clock tariff. A local day can contain 92, 96 or 100 slots. In October, two observations labelled 02:15 can represent different instants and different prices.

## Assets, and what an asset is not

An `Asset` is a thing behind the connection point with an identity, a rating and
a set of capabilities. The interesting ones are not the obvious ones.

| Asset | The part that is easy to get wrong |
|---|---|
| `Battery` | asymmetric charge and discharge ratings, a **backup reserve** that is a promise, and round-trip efficiency that belongs in the *fill rate* rather than in the power |
| `Evse` | a charge point is off, or at **at least 6 A per conductor** (IEC 61851). There is nothing in between, and a plan that trickles 500 W into it delivers nothing |
| `HeatPump` | consumer control interface, compressor and auxiliary heater kW, cooling rating, and comfort band |
| `DhwTank` | a store, not a second thermal model: three hundred litres between 45 and 60 °C are five kilowatt-hours of heat that can be bought hours before they are used. All five facts are the **installation's** — volume, heater, coefficient (1,0 for an immersion heater, 3,0 for a hot-water heat pump), standing loss, and the three temperatures. `t_set_c` is the one that is easy to misread: it bounds nothing the plan does, and it is where the household's own thermostat would sit with no manager at all — so it is what the *baseline* is held at, and therefore what the saving is measured against |
| `PvArray` | its § 9 EEG status is a **declaration** about the installation, not an inference from its nameplate |
| `FlexibleLoad` | `LoadKind::Shiftable` **carries** the `Programme` it will run, quarter hour by quarter hour |

That last one is the clearest case of a fact the type system can make
unrepresentable. A dishwasher's programme is *shaped* — two kilowatts to heat,
two hundred watts to wash, two kilowatts again to dry. An appliance that
announces flexibility and cannot say what shape it takes has told a manager
nothing it can act on, so the shape is not optional.

### Wiring and mode are different questions

`PhaseConnection` is what a device is **wired to**. `PhaseMode` is what it is
**using right now**. Answering the second with the first is how an 11 kW wallbox
drawing symmetrically on three conductors acquires the 4,6 kVA unbalance limit
that only applies to a single-phase device — and loses two thirds of its
charging power to a rule that was never about it.

## The electrical tree

A house is not a single busbar. A wallbox in the garage sits behind a
sub-distribution board with its own cable and its own fuse, and everything behind
that board is bounded by it — independently of, and usually well below, the main
connection.

<pre class="mermaid">
flowchart TD
  R["Grid connection<br/>63 A · both directions"] --> SB1["Sub-board: garage<br/>32 A"]
  R --> SB2["Sub-board: utility room<br/>25 A"]
  R --> H["Household load<br/>uncontrollable"]
  SB1 --> EV["Wallbox 11 kW"]
  SB2 --> HP["Heat pump 8 kW"]
  SB2 --> DHW["Hot-water tank 2 kW"]
  R --> BAT["Battery 5 kW"]
  R --> PV["Roof 9,8 kWp"]
</pre>

`Circuits` form a tree rooted at the grid connection, and the guard narrows every
asset's interval by **every** limit on its path to the root. That is the same
mechanism the § 14a and § 9 EEG limits use — they are simply limits that sit at
the root. Load management that only knows the main fuse either trips the
sub-board or leaves capacity unused, and there is no third option.

## Envelopes: every layer narrows, nobody widens

Every layer of hems hands down an **interval** rather than a number.

<pre class="mermaid">
flowchart LR
  A["Physically possible<br/>−5 … 11 kW"] --> B["Guard: fuses, ratings,<br/>reserve, § 14a, § 9 EEG"]
  B --> C["Plan: what it wants<br/>this quarter hour"]
  C --> D["Arbiter: a point<br/>inside what is left"]
  D --> E["Device: what it will<br/>actually accept"]
</pre>

The idea is OpenEMS's — a scheduler that hands each controller an interval of
possible solutions that later controllers can only shrink — made explicit. An
`Envelope` has a `floor` and a `ceiling`, both of which may be either sign, and
an **empty** interval is a real outcome rather than a bug: two rules that cannot
both be satisfied. `Envelope::resolve` keeps the stricter *ceiling*, because
exceeding a grid limit is worse than falling short of a floor.

This is what makes “the grid limit was respected” an intersection rather than a
code path somebody has to remember to write — see
[Architecture](@/docs/architecture.md#the-guard).

## A command cannot exist without a reason

Each setpoint carries a reason. Reasons identify the authority responsible for the request so a household can trace a command to its source.

| Authority | Example |
|---|---|
| `Guard` | A grid reduction or equipment limit |
| `User` | A household pause or charging request |
| `Cem` | An instruction from a named external energy manager |
| `Plan` | The current slot’s battery target |
| `Realtime` | Tracking the slot’s energy through a change in sunlight |
| `Fallback` | Local self-consumption without a current plan |

A production ceiling lets an inverter generate any power up to the specified maximum. A target asks an asset to track a particular value. Drivers declare which forms they can deliver.

### Non-finite values stop at the boundary

The unit constructors are infallible and cheap, and `debug_assert!` finiteness.
The gate that matters is where a number becomes an **action**: `Setpoint::new`
refuses a non-finite command, so a NaN out of a broken driver or a degenerate
solve can never reach a device.

## The building is a store, and it is discretised exactly

`ems-core::thermal` carries a two-mass RC model of the house: an air node with a
time constant of about thirteen minutes, and a fabric node with one measured in
days. The fabric usually stores several times what the household battery does,
and it is free.

It is stepped by an **exact zero-order hold** — the coefficients come from a
matrix exponential — rather than by explicit Euler. At a quarter-hour step the
explicit scheme gets the fast eigenvalue's *sign* wrong and the input gain 64 %
too large, and is only conditionally stable. The exact step is a contraction at
any step size, carries no discretisation error under the assumption a
quarter-hour plan already makes, and is still linear in the heat input — which is
what keeps the planner a linear program. [Planner thermal model](@/docs/optimizer.md#the-building-is-discretised-exactly-not-by-explicit-euler).

The same `Rc2Discrete` serves the planner, the rule-based baseline it compares
itself against, and the simulator that answers it. One model, and no way for the
plan and the house to disagree about physics for numerical reasons.

### The heat nobody paid for

The model carries two more numbers: an effective **solar aperture** and a
constant **internal gain** — the sun through the glazing, and the people, the
cooking and everything electric that ends up warm. Both are identified from the
household's own record along with the fabric.

They are first-order, not a refinement. The average archetype loses
`(21 − T_out) / 6` kW, which is 2,7 kW at 5 °C outdoors; a clear March noon puts
about 550 W/m² on a vertical south plane, and 4,5 m² of aperture turns that into
2,5 kW, with the internal gains another 0,45. **On the days a heating plan is
most worth making, the free heat exceeds the demand**, and in January it is about
half of it. A plan without it heats a room the sun is already warming and pays
the comfort slack for the overshoot — the opposite of the way a conservative
omission would be wrong.

It costs the programme nothing: the state equation is already linear in the heat
input, and free heat is a *known constant* per slot rather than a decision, so it
is an offset on the right-hand side.

## Everything takes time as a parameter

No function in `ems-core` reads a clock, opens a socket, spawns a task or
contains `unsafe`. `just purity` fails the build if it — or any of the other nine
domain crates — reaches for one. That is what makes a whole winter day, DST
transition included, a unit test that runs in milliseconds.

```rust
use ems_core::prelude::*;

let limit = Setpoint::new(
    AssetId::new("wallbox-garage")?,
    Command::ConsumptionCeiling(Power::from_kw(4.2)),
    Reason::guard(GuardRule::Lpc),
    time::macros::datetime!(2026-01-15 17:04:00 UTC),
)?;

assert_eq!(limit.authority(), Authority::Guard);
```
