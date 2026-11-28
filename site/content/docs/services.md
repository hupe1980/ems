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
| `fleetd` | Enrolment, desired and running configuration, signed releases and gateway attention desk | Credential and tenant |
| `portald` | Household energy history and charging receipts | Current household |
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

`fleetd` distinguishes configuration offered to a gateway from configuration the device reports as running. Sign in to `/desk` with your organization account. Filter to sites needing attention or search by site name. The desk link preserves search, status and page for a colleague with access. Expand a site and choose **Feeder reading age**, **Report age** or **Delivery delay**. Each point is one check-in, positioned at its UTC send time. Squares mark stale readings. Compare a rising feeder age with a steady delivery delay to investigate a meter that stopped updating while the gateway kept reporting. The desk retains the latest 32 check-ins per site. Install signed releases through the installer or operating system. [Trust and updates](@/docs/security.md).

<a href="../../screens/operator-desk.png"><img src="../../screens/operator-desk.png" width="800" style="max-width:100%;height:auto" loading="lazy" alt="Operator desk showing a feeder reading growing stale across successive gateway check-ins"></a>

### Investigate meter history

Open **Metered energy**, choose a period ending date and select 7, 30 or 90 days. Each bar represents a quarter-hour of grid exchange, with import above zero and export below. Drag across a week to zoom, or use **Peak import** and **Zoom to day**. The cursor reads both directions in kWh.

<a href="../../screens/meter-history.png"><img src="../../screens/meter-history.png" width="800" style="max-width:100%;height:auto" loading="lazy" alt="Thirty days of grid exchange with gaps in the final week and the exact reading at the highest import"></a>

Choose **First gap** to find a missing reading. **Open meter day** brings the selected quarter into focus. Use the arrow keys to inspect adjacent readings or open the table for exact kWh and Berlin times with their UTC offset. **Return to range** restores the earlier zoom and quarter.

<a href="../../screens/metered-energy.png"><img src="../../screens/metered-energy.png" width="800" style="max-width:100%;height:auto" loading="lazy" alt="Operator meter day with directional import and export bars and a gap in the morning readings"></a>

`obsd` authenticates signed day reports before accepting them. Fleet summaries keep outcome counts alongside averages. A single missed reduction warrants investigation even if the fleet-wide average is high. Coverage and missing quantities affect which sites can contribute to a cost or forecast metric. [Agent access](@/docs/agents.md).

### Review a gateway update rehearsal

Run `just canary-trial` for a disposable fleet of 100. The operator desk shows the selected canary, its verified candidate and the build that reported ready after restart. Readiness expires after 120 seconds. A healthy gateway running the previous build leaves the next simulated site on hold.

<a href="../../screens/update-rehearsal.png"><img src="../../screens/update-rehearsal.png" width="800" style="max-width:100%;height:auto" loading="lazy" alt="Operator desk showing the candidate digest, running build and readiness of a simulated gateway"></a>

### Household departure receipt

Sign in to `/portal` with your household account. Choose **Grid import**, **Grid export** or **Generation** and a 7- or 30-day period ending on the latest report. Select a date for its reading, or jump to the highest reported day or latest gap. The scale stays fixed when switching measures within the period. **No report** marks a missing day. Open **Daily readings** for the underlying kWh values. [Configure browser sign-in](https://github.com/hupe1980/ems/tree/main/deploy/oidc).

The reference household requests 6.00 kWh. When the outlet closes at 4.75 kWh delivered, the portal shows **1.25 kWh short**. Expand **Outlet meter readings** for the start, reconnection and departure registers, or download the receipt as JSON. If the departure reading is missing, the receipt shows what was confirmed at reconnection. [Rehearse a departure](https://github.com/hupe1980/ems/tree/main/services/portald).

<a href="../../screens/household-portal.png"><img src="../../screens/household-portal.png" width="560" style="max-width:100%;height:auto" loading="lazy" alt="Household portal showing a week of energy history and a charging receipt 1.25 kWh short"></a>

## Running the fleet locally

```sh
just fleet-demo
```

The demo starts a disposable PostgreSQL container and collector on loopback, generates a household signing key, and submits a signed day from the gateway simulator. It checks that the household appears in the fleet view and removes the database when it exits. For a persistent installation, start each daemon with its own example as a template and configure the required store and credentials. Secret references such as `env:EMS_OBSD_SECRET_REFERENCE_HOUSEHOLD` name environment variables whose values the process reads at runtime.

For deployment monitoring, use `/livez` to detect a stalled process, `/readyz` to check dependencies and `/metrics` for Prometheus metrics. A running process can still be unable to serve a request if its database is unavailable.

## Historian register storage

Configure `[register_meters.<site>]` with the market location (`malo`) and responsible market participant (`operator`) before the first submission. For that site, `histd` uses MeterStore’s PostgreSQL hot tier for register ingestion, settlement, and historical exports. The historian stamps each accepted revision. At a correction’s receipt time, the corrected values become visible. Earlier exports retain their original values. Verify locally with `just histd-registers`.
