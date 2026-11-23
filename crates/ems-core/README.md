# ems-core

Shared types for sites, meters, assets, circuits, measurements and reasoned commands. Schedules use 15-minute slots with Europe/Berlin calendar boundaries. Time enters as a parameter.

Positive power flows into the measured asset. Consumption is positive and generation is negative. At the connection meter, positive power means grid import. The site balance can therefore compare grid power directly with the sum of asset powers.

A `Setpoint` includes its source and the reason for the decision. `Envelope::resolve` gives the stricter ceiling precedence when a requested floor exceeds it. Device wiring and the number of conductors currently in use are represented separately.

```rust
use ems_core::prelude::*;

let limit = Setpoint::new(
    AssetId::new("wallbox-garage")?,
    Command::ConsumptionCeiling(Power::from_kw(4.2)),
    Reason::guard(GuardRule::Lpc),
    time::macros::datetime!(2026-01-15 17:04:00 UTC),
)?;
assert_eq!(limit.authority(), Authority::Guard);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Use it

```console
$ cargo add ems-core
```

API reference on [docs.rs](https://docs.rs/ems-core); the design is explained in
[The domain model](https://hupe1980.github.io/ems/docs/domain-model/).

## License

MIT OR Apache-2.0
