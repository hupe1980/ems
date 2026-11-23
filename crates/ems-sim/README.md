# ems-sim

Deterministic models of household devices, weather and demand for exercising a controller. Supply an instant and a step duration. Seeded weather lets two runs see the same cloud cover, temperature and load.

| Model | Behaviour visible to the controller |
|---|---|
| Battery and inverter | State-of-charge bounds, conversion and standing losses, and the measured response to a curtailment command. |
| EV charging | Minimum current, available conductors, phase switching and charging efficiency. |
| Heating and hot water | Building temperature, thermostat control, compressor minimum runtime and tank draw. |
| Appliances and grid control | A fixed programme can continue after a stop request. A scripted control box can impose and release a limit. |

Use the household simulator to compare the plan with the uncontrolled baseline under the same realised weather. The planner normally receives a forecast. The `--perfect-foresight` flag deliberately exposes the realised weather for comparison.

```sh
cargo run --release -p hemsd -- simulate --day winter
```

Enable the `peak` feature for industrial replay. `ems_sim::peak` projects battery dispatch over a quarter hour and reconciles closed meter intervals against the cumulative import register. The [reference trace](fixtures/shift-trace.json) spans the repeated autumn clock hour.

[API reference](https://docs.rs/ems-sim) · [Guide](https://hupe1980.github.io/ems/docs/simulation/)

## License

MIT OR Apache-2.0
