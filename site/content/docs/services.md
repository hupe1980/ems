+++
title = "Fleet services"
description = "Choose the service for prices, weather, site records, gateway enrolment or fleet monitoring."
weight = 13

[extra]
group = "Integration and operations"
+++

The household gateway can enforce local limits without a fleet connection. Install services according to the data you need to supply, retain or aggregate. Each service ships its own example configuration.

| Service | Use it for | Scope |
|---|---|---|
| `tariffd` | Day-ahead prices, carbon intensity and published network charge windows | Public market inputs |
| `forecastd` | Quarter-hour weather and a production estimate for a supplied geometry | Weather for a configured location |
| `histd` | Meter registers, grid-control evidence and settlement exports | Authorized household site |
| `fleetd` | Enrolment, desired and running configuration, signed releases and operator roster | Credential and tenant |
| `obsd` | Signed day reports, forecast quality and operating incidents | Authorized reporting site or operator |
| `agentd` | An advisory view across permitted services | Proposals for review |

## `tariffd` — the price stack

Use `/v1/prices` for the requested time window and `/v1/prices/coverage` to inspect coverage. Carbon intensity and operator-specific Modul 3 windows have separate routes. The gateway combines these inputs with its effective retail contract. An absent wholesale price and a price of zero lead to different settlement outcomes. [Tariffs and contracts](@/docs/tariffs.md).

## Weather and site forecasts

`forecastd` serves quarter-hour weather at `/v1/weather/{location}` and a production estimate at `/v1/production/{location}`. The household gateway requests the weather and models its own array using site orientation, installed inverter capacity and local measurements. [Household forecasts](@/docs/forecasting.md).

## Site history and settlement

`histd` stores grid events and quarter-hour registers separately. An evidence export answers when a reduction arrived and what the gateway observed. A settlement export accounts for metered energy over a billing period. Site credentials restrict writes to the site they represent.

Use `/v1/sites/{site}/quarter-hours` to inspect the incoming registers, `/v1/sites/{site}/nachweis` for control evidence, and `/v1/sites/{site}/mispel` for the configured settlement method. Settlement depends on the site’s declared method, complete quantities and applicable prices. Measured energy survives missing price data so that an incomplete financial result can still be investigated. [Grid rules](@/docs/grid-rules.md) · [Service configuration](https://github.com/hupe1980/ems/tree/main/services/histd).

A register has three times: the start of its measurement interval, the gateway’s revision time (`recorded_at` in the POST body), and the time the historian accepted it. When a gateway reconnects, a late correction changes the current settlement. Save the settlement’s `as_of` value to retrieve the same register versions later. The service returns its effective cutoff even on the first request. An omitted or future cutoff uses the database clock. Retry a lost acknowledgement with the original revision time. HTTP 409 means the same revision already has different values; the entire batch was refused. [Historian README](https://github.com/hupe1980/ems/tree/main/services/histd).

## Fleet administration and monitoring

`fleetd` distinguishes configuration offered to a gateway from configuration the device reports as running. Operators can inspect their tenant’s fleet and release manifests. Gateways verify signed release material before applying it. [Trust and updates](@/docs/security.md).

`obsd` authenticates signed day reports before accepting them. Fleet summaries keep outcome counts alongside averages. A single missed reduction warrants investigation even if the fleet-wide average is high. Coverage and missing quantities affect which sites can contribute to a cost or forecast metric. [Agent access](@/docs/agents.md).

## Running the fleet locally

```sh
just fleet-demo
```

This starts the local observability example and submits a signed household simulation report. For a persistent installation, start each daemon with its own example as a template and configure the required store and credentials. Secret references such as `env:EMS_OBSD_SECRET_REFERENCE_HOUSEHOLD` name environment variables whose values the process reads at runtime.

For deployment monitoring, use `/livez` to detect a stalled process, `/readyz` to check dependencies and `/metrics` for Prometheus metrics. A running process can still be unable to serve a request if its database is unavailable.

## Historian register storage

Configure `[register_meters.<site>]` with the market location (`malo`) and responsible market participant (`operator`) before the first submission. For that site, `histd` uses MeterStore’s PostgreSQL hot tier for register ingestion, settlement, and historical exports. The historian stamps each accepted revision. At a correction’s receipt time, the corrected values become visible. Earlier exports retain their original values. Verify locally with `just histd-registers`.
