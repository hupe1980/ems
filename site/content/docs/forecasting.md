+++
title = "Household energy forecasts"
description = "How the gateway learns rooftop output and household demand, and how to read its forecast scores."
weight = 9

[extra]
group = "Control and metering"
+++

The planner needs next morning’s sunlight and household demand before either appears on a meter. The gateway combines weather and site geometry with measured household history. Each forecast band gives a range of plausible outcomes. A wide band means that committing a store on the median prediction alone carries more risk.

| Input | How the gateway estimates it |
|---|---|
| Rooftop PV | Solar geometry, irradiance, panel orientation and temperature, inverter conversion and clipping, then a correction learned from the local roof |
| Household load | Quarter-hour history grouped by day type, with a prior for sparse observations |
| Car availability | Arrival, departure and charge from weekday session history, superseded by fresh device observations |
| Building temperature | A thermal model fitted to informative periods with indoor and outdoor temperature and heating data |

## Solar production and roof performance

An inverter loses proportionally more power at low load and clips above its AC rating. Both effects matter on a small roof around dawn and on clear summer days. The gateway learns a multiplicative production correction by local hour, so regular morning shading changes the forecast for that part of the day.

A slower monitor compares recent roof performance with a longer baseline. This can reveal a persistent loss that an adaptive forecast has already learned. Its irradiance denominator is modelled. Treat a change as a reason to inspect the installation. An independent irradiance sensor and meter are needed for an instrumented performance assessment.

## The first evening

A new gateway estimates gross household demand using a synthetic morning and evening load shape on the local holiday calendar. Set `[site] household_kwh_per_year` to annual appliance consumption, excluding heat, EV charging and storage. Without that figure the gateway starts from a bounded meter reading and fits the scale once all 96 quarters of a day type have readings. After four observations of a quarter and day type, its measured history supplies the entire forecast for that cell. Learning survives restarts.

## Reading forecast quality

The Day and Month views compare forecasts with observed outcomes when sufficient measurements cover the quarter. Incomplete PV or connection-meter coverage leaves the interval unscored. For solar production, the simulation reports the number of lit slots alongside the score so that predictable darkness cannot inflate apparent accuracy.

| Metric | Interpretation |
|---|---|
| Bias | Whether a forecast systematically runs high or low |
| Mean absolute error | Typical magnitude of the error in the measured unit |
| Coverage | Share of outcomes inside a stated prediction band |
| CRPS | Continuous ranked probability score, which measures the error of the predicted distribution in the original unit. Lower is better. |

A day’s quarter-hours share weather and occupancy patterns. Judge calibration across multiple independent days or seeded episodes. A simulated score describes those scenarios. Field evaluation requires held-out measurements from the installation.

## Reproduce a comparison

```sh
cargo run -p hemsd -- simulate --day winter
cargo run -p hemsd -- simulate --day winter --perfect-foresight
cargo run -p hemsd -- backtest --day summer --days 20
```

The first two runs use the same simulated weather and household. Perfect foresight gives the planner the realised inputs. Compare their costs and check that the unmanaged baseline stays the same. The backtest varies the seed across episodes to evaluate the forecast band. [Simulation and evaluation](@/docs/simulation.md#one-day-cannot-answer-some-questions).
