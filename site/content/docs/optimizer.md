+++
title = "Household energy planner"
description = "How the planner schedules charging, storage and heat within household goals and grid limits."
weight = 8

[extra]
group = "Control and metering"
+++

The planner chooses when to use grid electricity, solar generation and stored energy to meet household goals. By default it considers 192 quarter-hours. It sends the current slot’s targets to the arbiter and updates future slots when the next plan is solved. A household can see how changing an EV departure or hot-water deadline affects its plan. [Household Plan](@/docs/household.md).

## The formulation

Each slot balances grid import, export, rooftop production, curtailed generation, battery flows and device demand. Storage equations carry energy forward with charging losses, discharge losses and reserve constraints. The optimizer minimises total cost in euros, including purchased energy, export revenue, battery throughput, curtailed generation, missed service and the residual value of stored energy.

| Asset or constraint | What the model accounts for |
|---|---|
| Battery | Power and energy ratings, reserve, asymmetric losses, wear and the value of energy remaining at the horizon |
| EV | Connection window, charger minimum current, installed phase count, energy requested at departure and the cost of a shortfall |
| Heating | Thermal storage in the building and tank, comfort bounds, hot-water draw and compressor minimum runtime |
| Shiftable appliance | Feasible start times and the programme’s measured or declared sequence of loads |
| Grid connection | Shared import and export limits in the slots where they apply |

Charging a battery can look profitable during a cheap quarter-hour and still lose money. The later discharge must recover both conversion losses and the cost of cycling the pack. Similarly, moving heat across a tariff boundary earns a benefit only if room comfort or hot water is still delivered.

### The § 14a constraint is on the netzwirksamer Leistungsbezug, per slot

The site’s declared equipment and commissioning facts determine which devices share a reduction. The planner applies the relevant ceiling in each affected slot and accounts for eligible on-site production. The runtime guard recalculates the permitted power from current measurements each tick. [Grid rules](@/docs/grid-rules.md).

### The building is discretised exactly, not by explicit Euler

The thermal model advances room and building temperatures under constant inputs over a slot. Exact discretisation preserves the response of the continuous model at the chosen step size. Remaining heat carries a terminal value so that the last slot does not reward leaving a cold building. [Thermal storage model](@/docs/domain-model.md#the-heat-nobody-paid-for).

## § 42c: the neighbours' roof as a cheap block of import

When configured, community energy is a limited block of import at a separate price in the relevant quarter-hour. The retail price stack still includes applicable network charges and taxes. Both the planned household and its baseline receive the same allocation opportunity. [Tariff inputs](@/docs/tariffs.md).

## Solvers

`ems-optimizer` supports the pure-Rust microlp backend and HiGHS. The default `hemsd` build enables HiGHS and requires CMake and a C++ compiler. The gateway’s default solve budget is ten seconds. A wall-clock budget can change the best feasible result found on a busy machine. Reproducible scenario runs can disable the time limit.

## What the plan carries

Each modelled asset gets a target and an envelope describing how far the arbiter can move it. Once the integer decisions are fixed, a continuous solve can estimate the marginal value of energy in each store. For instance, one additional kilowatt-hour can be more valuable to a car just before departure than to a battery with an entire cheap night ahead.

Use a reference day to inspect the result and compare the delivered service, costs and binding restrictions. The simulator reports its assumptions alongside the outcome. [Simulation and evaluation](@/docs/simulation.md#winter-report).

```sh
cargo run -p hemsd -- simulate --day winter
```
