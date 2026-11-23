# ems-tariff

Build a household’s import and feed-in prices for each quarter hour. The stack keeps wholesale energy, supplier charges, grid fees, levies and tax separately so the planner and bill can use the same inputs.

- Money uses decimal cents per kWh. Keep the raw spot price as well as the retail price when evaluating § 51 EEG.
- The Modul 1, 2 and 3 comparison uses the household’s meter history. The Modul 2 break-even result is an annual energy threshold.
- Parsers support ENTSO-E A44 XML, SMARD, aWATTar, Tibber and Energy-Charts. Hourly prices expand to four slots. Timestamp-based indexing handles the short and long local days at daylight-saving transitions.
- With the default `serde` feature, `sheet::PriceSheet` reads a version-1 seller sheet. Try the [fixed](examples/preisblatt-fixed.json) or [dynamic](examples/preisblatt-dynamic.json) fixture from the workspace root.

```sh
cargo run -p hemsd -- price-sheet crates/ems-tariff/examples/preisblatt-fixed.json --at 2027-01-05T17:00:00+01:00
```

## Use it

```console
$ cargo add ems-tariff
```

API reference on [docs.rs](https://docs.rs/ems-tariff); the design is explained in
[Tariffs and prices](https://hupe1980.github.io/ems/docs/tariffs/).

## License

MIT OR Apache-2.0
