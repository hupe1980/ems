# ems-optimizer

Build a quarter-hour schedule for a household with PV, battery storage, an EV, heating, hot water or shiftable appliances. The gateway executes the first slot, then solves again as measurements and forecasts change.

Pass a `Problem` to `solve`. It includes the horizon, price and forecast series, asset state, deadlines and grid limits. Each series must cover the requested slots. The objective prices energy, battery throughput, discomfort and missed service in euros.

- EV charging is either off or above the minimum current. A programme such as a dishwasher must fit its complete power profile into the start window.
- The thermal model carries building temperature, tank energy and outstanding compressor runtime between plans.
- Scenario-based planning can share the first decision across several forecast outcomes and price downside risk using conditional value at risk.
- A second solve fixes the discrete choices and calculates the marginal value of each store and the grid constraint. The runtime uses those values when allocating limited power.

The default mixed-integer solver is pure-Rust `microlp`. Enable `highs` for HiGHS, which requires CMake and a C++ compiler. Both configurations use Clarabel for the marginal values.

To exercise the planner with a complete household from the workspace root:

```sh
cargo run --release -p hemsd -- simulate --day winter
```

[API reference](https://docs.rs/ems-optimizer) · [Guide](https://hupe1980.github.io/ems/docs/optimizer/)

## License

MIT OR Apache-2.0
