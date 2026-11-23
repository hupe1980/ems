# hemsd

A household energy gateway with a local control loop, planner and browser UI. The gateway uses SunSpec/Modbus TCP and EEBUS to communicate with devices and exposes S2 to an external energy manager.

## Try the interface

```sh
cargo run -p hemsd -- demo --days 1
```

Open the [household page](http://127.0.0.1:8080/). **Now** explains what the devices are doing, **Plan** lets you price a departure or hot-water goal, and **Day** shows the energy and cost for a closed day.

## Run a gateway

```sh
cargo run -p hemsd -- run --config house.toml
```

With an empty or missing file, sign in with the printed access code, identify the three-phase SunSpec connection meter, and review the fuse and tariff before confirming. PV and battery observation also require the device nameplate and an identified controller enforcing site limits.

For a configured site, use `hemsd run --check --config house.toml` to validate the file before opening connections. The gateway serves its page and API over HTTPS when bound off loopback. Local evidence and its delivery outbox use `redb`. [Example configuration](hemsd.example.toml) · [Installation guide](https://hupe1980.github.io/ems/docs/getting-started/)

## Local power history

Enable `[series]` in the gateway configuration. In **Day**, inspect a quarter hour at the grid connection to see mean power, the highest and lowest readings, and how many observations the meter supplied. The JSON download includes each measurement point and identifies any failed reads. [History API](https://hupe1980.github.io/ems/docs/api/#local-power-history)

## Gateway diagnostics

Run `hemsd doctor --config house.toml` with the household credential in `HEMS_TOKEN` or `[api].token`. For private HTTPS, add `--ca-cert hems.pem`. Doctor checks device responses, Steuerbox trust and the control clock. Setup shows the last accepted LPC heartbeat. When stored quarters need recalculation, doctor and Setup show a history warning until automatic maintenance finishes. A pending or unreadable check exits nonzero.

## Observe an installation

Set `mode = "shadow"`, configure `external_limit_controller` and set a local `store_path`. The gateway reads the connection meter and previews the plan. Once the meter and actuator paths have been verified, the household can authorize installer handover for one hour. [Observation and receipts](https://hupe1980.github.io/ems/docs/household/#observe-a-household)

## Seller price sheets

Run `hemsd price-sheet FILE --at 2027-01-05T17:00:00+01:00` for the gross import price at one instant. Add the net index with `--index-ct` for dynamic contracts. Use `--signed` for an Ed25519 JWS. Setup shows the signer fingerprint and asks the household to approve the effective contract. [Worked example](https://hupe1980.github.io/ems/docs/tariffs/#check-a-seller-sheet-locally)

## Matter appliance power

Build with `cargo build -p hemsd --features matter`. Select a commissioned appliance in the local Matter Server and run `hemsd matter-read --config house.toml` for a single JSON observation. Setup displays signed watts and the last successful read. Run `just matter-recovery` to exercise peer loss and gateway restart against a simulated Matter Server over its WebSocket API. [Configuration and reading states](https://hupe1980.github.io/ems/docs/household/#matter-appliance-power)

## Exercise a controller

| Command from the workspace root | What it exercises |
|---|---|
| `cargo run --release -p hemsd -- simulate --day winter` | The planner, guard and arbiter against simulated devices and seeded weather. Add `--json` for machine-readable results. |
| `cargo run -p hemsd --features house -- house --run --at 16:58` | A gateway connected to a virtual inverter, battery, meter and Steuerbox over local sockets. |
| `cargo run --release -p hemsd -- backtest --day summer --days 20` | Forecast coverage across independent simulated days. |
| `just quote-trial` | Slider-to-price latency over 20 accepted departure goals. |
| `just retained-history-trial` | Export during virtual control with 1,095 days of stored quarters. |

For API integration, `hemsd openapi` emits the schema and `cargo xtask check-api` verifies it against the checked-in contract. [OpenAPI 3.1](openapi.json) · [Device capabilities](https://hupe1980.github.io/ems/docs/devices/)

## License

MIT OR Apache-2.0
