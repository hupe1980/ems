# cemsd

The daemon serves the industrial replay from `ems_sim::peak`. Try the [peak challenge](https://hupe1980.github.io/ems/peak-challenge/) in your browser, or start the local replay to explore meter gaps, battery limits and the repeated clock hour.

```sh
cargo run -p cemsd -- demo
```

Open the [shift replay](http://127.0.0.1:8082/). Choose **Jetzt starten** or **Zwei Minuten warten**, then **Zählerabschluss aufdecken**. A 120 kW battery can deliver 10 kWh in five minutes and 6 kWh in three minutes. The two choices project 1,100 kW and 1,116 kW respectively. Both reveal the same 285 kWh meter close, a 1,140 kW mean with the battery idle.

The recording crosses the autumn clock change in Berlin. Check the UTC offset alongside the local time. **Letzter möglicher Start** shows how late this battery can still keep the quarter at or below the previous peak. [Delayed decision as JSON](http://127.0.0.1:8082/v1/demo/industrial?moment=delayed) · [Reveal its meter close](http://127.0.0.1:8082/v1/demo/industrial?moment=delayed&reveal=true)

Compare a local cumulative register with independently closed quarter hours offline. The report preserves both repeated local hours and reports the largest difference in Wh.

```sh
cargo run -p cemsd -- reconcile crates/ems-sim/fixtures/shift-trace.json
```

In the Linux archive, run `./cemsd reconcile examples/shift-trace.json` from the extracted directory.

## Rehearse an OpenADR event

```sh
cargo run -p cemsd --features openadr -- openadr
cargo run -p cemsd --features openadr -- openadr --cancel-before-dispatch --json
```

Each command starts a fresh, authenticated OpenADR 3.1 VTN on loopback. The simulated site reads its event and projects both battery start times against the requested 1,100 kW quarter-hour mean. At close, the site reports the original battery-idle meter’s 285 kWh to the VTN and reads the report back. Withdrawal clears the request before either dispatch. Add `--json` to either run for the numeric receipt.

The private program defines `EMS_QUARTER_MEAN_LIMIT` in `KW` over 15 minutes. Its meter report uses `USAGE` in `KWH` with `DIRECT_READ`. The optional feature uses the upstream wire model, timeline and in-memory VTN with a ten-second loopback HTTP client that disables proxies and redirects. [OpenADR crate](https://github.com/hupe1980/openadr/tree/v0.6.0)
