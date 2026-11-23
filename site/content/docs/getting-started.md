+++
title = "Getting started"
description = "Run the home, building or industrial demo, validate a configuration and check a gateway."
weight = 1

[extra]
group = "Start"
+++

## Quickstart

To try EMS without installing a gateway, open the [industrial peak challenge](@/peak-challenge.md) and choose when the battery starts.

Install the Rust toolchain pinned in `rust-toolchain.toml`. Building the household gateway also requires CMake and a C++ compiler for HiGHS.

```sh
git clone https://github.com/hupe1980/ems
cd ems
```

Choose a demo and open the printed local address.

| Environment | Command | First task |
|---|---|---|
| Home | `cargo run -p hemsd -- demo` | Open Plan on port 8080 and compare two departure goals. |
| Building | `cargo run -p bemsd -- demo` | Open the handover on port 8081 and step from the request to the arrival of the last station reading. |
| Energy community | `cargo run -p bemsd -- community --viewer member-a` | Sign in on port 8083 with the printed login and check your share for the second 02:00 quarter. |
| Industrial site | `cargo run -p cemsd -- demo` | On port 8082, compare the battery decision at 02:55 with waiting until 02:57 before the autumn clock change. |

The demos use synthetic inputs and run on loopback. To compare an industrial meter trace with closed intervals, use the [reconciliation guide](@/docs/simulation.md#check-a-shift-against-its-meter).

## Watch a day

Run `cargo run -p hemsd -- simulate --day winter` for the January energy and cost report. Use `cargo run -p hemsd -- demo --days 30` to explore a month of receipts in the browser. [Winter report and reference days](@/docs/simulation.md#winter-report).

## A house without hardware

```sh
cargo run -p hemsd --features house -- house --run --at 16:58
```

The virtual inverter, meter and battery answer real Modbus TCP requests. A simulated Steuerbox sends a reduction at 17:00. Open the printed household URL and sign in with the code from the terminal. The command writes `house.toml` for this installation. [Household controls](@/docs/household.md).

## Manage a real house

Copy `services/hemsd/hemsd.example.toml` and set the site, installed assets, driver addresses and a writable store path. Enter the roof orientation and battery reserve before comparing plans. Validate the file, then start the gateway.

```sh
cargo run -p hemsd -- run --check --config house.toml
cargo run -p hemsd -- run --config house.toml
```

Check device health in Setup or run `hemsd doctor --config house.toml` with the household credential. For a private HTTPS certificate add `--ca-cert hems.pem`. [Drivers](@/docs/devices.md) · [Trust and signing in](@/docs/household.md#signing-in-and-the-network-it-runs-on) · [Local API](@/docs/api.md).

Setup and doctor show the time since the last accepted Steuerbox heartbeat. For monitoring, alert on `hems_control_box_needs_attention == 1`.

## Run the checks

Install `just`, `cargo-deny` and a container runtime, then run `just ci`. It checks formatting, Clippy, domain-crate purity, tests, workspace contracts, packaging, dependencies and rustdoc. The database tests share a local PostgreSQL container. Set `EMS_TEST_POSTGRES` to use an existing test server.

For the browser regression run, start all three edge demos on their default ports, then run `node xtask/media.mjs release-preview` with Node 24 and Chrome installed. Set `CHROME` to the browser executable on Linux. [Workspace architecture](@/docs/architecture.md) · [Fleet services](@/docs/services.md).
