# hemsd

A household energy gateway with a local control loop, planner and browser UI. The gateway uses SunSpec over Modbus TCP or RTU and EEBUS to communicate with devices and exposes S2 to an external energy manager.

On first start the gateway uses a synthetic household load profile. Set `[site] household_kwh_per_year` to annual appliance consumption when known. [Forecasting guide](https://hupe1980.github.io/ems/docs/forecasting/)

For fleet reporting, configure the same `whsec_<standard Base64>` signing key in `[obsd].secret` and in the collector’s entry for this site. [Key provisioning and rotation](https://hupe1980.github.io/ems/docs/security/#secrets-and-transport)

## Try the interface

```sh
cargo run -p hemsd -- demo --days 1
```

Open the [household page](http://127.0.0.1:8080/). **Now** explains what the devices are doing, **Plan** lets you price a departure or hot-water goal, and **Day** shows the energy and cost for a closed day. Select a day in Month to see the matched bill difference or the first service check that failed.

## Gateway image and event contracts

The `hemsd` Dockerfile target runs as UID 65532. Mount the gateway configuration at `/etc/ems/hemsd.toml` and persistent writable storage at `/var/lib/hemsd`. The gateway owns the household REST, structured CloudEvents and signed webhook ports. Run `just interfaces` from the workspace root to exercise a real delivery. [Published event schemas](../../schemas/events) · [Replacement guide](https://hupe1980.github.io/ems/docs/api/#verify-a-replacement)

## Run a gateway

```sh
cargo run -p hemsd -- run --config house.toml
```

With an empty or missing file, sign in with the printed access code, identify the three-phase SunSpec connection meter, and review the fuse and tariff before confirming. PV and battery observation also require the device nameplate and an identified controller enforcing site limits.

For an RS-485 connection meter, provision the serial port in `house.toml`, then select its bus and unit in Setup. Run `just serial-meter-trial` to rehearse the workflow against a simulated model 203 meter. [Serial meter setup](https://hupe1980.github.io/ems/docs/devices/#commission-a-serial-grid-meter)

For a wired heat meter, add a `[heat_meter]` table to the commissioned household configuration with the converter path, primary address, manufacturer and eight-digit meter ID. Setup and `hemsd doctor` report the cumulative delivered thermal Wh and flag a silent or swapped meter. [Configuration and supported meters](https://hupe1980.github.io/ems/docs/devices/#read-a-wired-heat-meter)


For a configured site, use `hemsd run --check --config house.toml` to validate the file before opening connections. The gateway serves its page and API over HTTPS when bound off loopback. Local evidence and its delivery outbox use `redb`. [Example configuration](hemsd.example.toml) · [Installation guide](https://hupe1980.github.io/ems/docs/getting-started/)

## Local power history

Enable `[series]` in the gateway configuration. In **Day**, inspect a quarter hour at the grid connection to see mean power, the highest and lowest readings, and how many observations the meter supplied. The JSON download includes each measurement point and identifies any failed reads. [History API](https://hupe1980.github.io/ems/docs/api/#local-power-history)

## Gateway diagnostics

Run `hemsd doctor --config house.toml` with the household credential in `HEMS_TOKEN` or `[api].token`. For private HTTPS, add `--ca-cert hems.pem`. Doctor checks device responses, Steuerbox trust and the control clock. Setup shows the last accepted LPC heartbeat. When stored quarters need recalculation, doctor and Setup show a history warning until automatic maintenance finishes. A pending or unreadable check exits nonzero.

## Observe an installation

Set `mode = "shadow"`, configure `external_limit_controller` and set a local `store_path`. The gateway reads the connection meter and previews the plan. Once the meter and actuator paths have been verified, the household can authorize installer handover for one hour. [Observation and receipts](https://hupe1980.github.io/ems/docs/household/#observe-a-household)

## Seller price sheets

Run `hemsd price-sheet FILE --at 2027-01-05T17:00:00+01:00` for the gross import price at one instant. Add the net index with `--index-ct` for dynamic contracts. Use `--signed` for an Ed25519 JWS. Try the checked-in `services/hemsd/tests/fixtures/tariff/dynamic.jws` with `--index-ct=-2` to compare the quarters immediately before and at 17:00 Berlin time. Setup shows the signer fingerprint and asks the household to approve the effective contract. [Worked example](https://hupe1980.github.io/ems/docs/tariffs/#check-a-seller-sheet-locally)

## Matter appliance power

Build with `cargo build -p hemsd --features matter`. Select a commissioned appliance in the local Matter Server and run `hemsd matter-read --config house.toml` for a single JSON observation. Setup displays signed watts and the last successful read. Run `just matter-recovery` to exercise peer loss and gateway restart against a simulated Matter Server over its WebSocket API. [Configuration and reading states](https://hupe1980.github.io/ems/docs/household/#matter-appliance-power)

## Exercise a controller

Build the simulation commands with `cargo build -p hemsd --features house`, then run them from `target/debug/`.

| Command | What to inspect |
|---|---|
| `hemsd house --run --with-room --with-tank --with-ev --manage-storage` | In **Now**, compare a battery command with measured power while the heaters and wallbox share the connection meter. |
| `hemsd winter-day --html > winter.html` | Inspect all 96 accelerated quarters, including the hot-water deadline and the closing battery charge. |
| `hemsd ev-departure --goal-wh 6000` | Read the OCPP outlet delivery and shortfall after a charger interruption. |
| `just house-meter-trial` | Reconcile meter registers and bills for winter and both Berlin clock changes. |

For a retained ledger and an independent run with the same installed appliances, follow the [winter simulation guide](https://hupe1980.github.io/ems/docs/simulation/#verify-a-vehicle-departure). For API integration, `hemsd openapi` emits the [OpenAPI contract](openapi.json).

## License

MIT OR Apache-2.0
