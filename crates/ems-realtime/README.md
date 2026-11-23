# ems-realtime

Compute one control tick from site measurements, the current plan, overrides and grid limits. The guard calculates the power interval available to each asset. The arbiter chooses a physically achievable command inside that interval and records its reason.

- The guard checks device ratings and battery reserve, then shares connection, sub-distribution and applicable grid limits among the devices using them.
- A § 14a reduction applies to controllable demand drawn from the grid. The configured attribution rule determines how local generation contributes under `[A1 2.3]`. The § 9 EEG limit applies to net export at the connection meter.
- A discharging battery can free capacity under `[A1 2.3]`. Credit is bounded by measured discharge, the requested battery power and the energy available over the control period.
- A silent controllable load is budgeted at its rating. A silent inverter contributes no PV headroom. The verdict identifies which assets required assumptions.

The caller supplies time. This lets the running gateway and a simulated site exercise the same fallback behaviour, including a missing plan or a silent Steuerbox.

## Use it

```console
$ cargo add ems-realtime
```

API reference on [docs.rs](https://docs.rs/ems-realtime); the design is explained in
[Architecture](https://hupe1980.github.io/ems/docs/architecture/).

## License

MIT OR Apache-2.0
