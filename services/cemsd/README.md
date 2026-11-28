# cemsd

Commission an industrial import meter and compare its register with independently closed quarter hours. Try the [peak challenge](https://hupe1980.github.io/ems/peak-challenge/) in your browser, or start the local replay to explore meter gaps, battery limits and the repeated clock hour.

```sh
cargo run -p cemsd -- demo
```

Open the [shift replay](http://127.0.0.1:8082/). Choose **Jetzt starten** or **Zwei Minuten warten**, then **Zählerabschluss aufdecken**. A 120 kW battery can deliver 10 kWh in five minutes and 6 kWh in three minutes. The two choices project 1,100 kW and 1,116 kW respectively. Both reveal the same 285 kWh meter close, a 1,140 kW mean with the battery idle.

The recording crosses the autumn clock change in Berlin. Check the UTC offset alongside the local time. **Letzter möglicher Start** shows how late this battery can still keep the quarter at or below the previous peak. [Delayed decision as JSON](http://127.0.0.1:8082/v1/demo/industrial?moment=delayed) · [Reveal its meter close](http://127.0.0.1:8082/v1/demo/industrial?moment=delayed&reveal=true)

Compare a local cumulative register with independently closed quarter hours offline. The report preserves both repeated local hours and reports the largest difference in Wh.

```sh
cargo run -p cemsd -- reconcile crates/ems-sim/fixtures/shift-trace.json
cargo run -p cemsd -- reconcile --format html crates/ems-sim/fixtures/shift-trace.json > review.html
```

Open `review.html` offline and follow **Go to first exception**. Each quarter shows its Berlin offset, UTC start, register increase, independent close and signed difference. JSON is the default. Valid exceptions produce a report and a nonzero exit. Invalid input produces only a diagnostic.

In the Linux archive, run `./cemsd reconcile examples/shift-trace.json` from the extracted directory.

## Commission an OPC UA import meter

Build with `cargo build --locked -p cemsd --features opcua`. Start with the supplied capture or collect one from an approved endpoint. The contract selects one scalar import register in Wh by namespace URI and node identifier. Confirm the meter’s direction, reset behavior and boundary latching against the site's point list.

From the workspace root, open an offline review of two quarters spanning Berlin’s repeated hour:

```sh
meter=services/cemsd/examples/opcua
target/debug/cemsd commission-meter --contract "$meter/contract.json" \
  --readings "$meter/readings.ndjson" --window "$meter/window.json" --format html > review.html
```

In the Linux archive, use `./cemsd` and `examples/opcua`. Omit `--format html` for JSON. If a quarter lacks a boundary or exceeds tolerance, the command still writes the report and exits nonzero. Invalid input produces a diagnostic on stderr.

To acquire fresh evidence, use the approved server leaf, trusted client application certificate and matching private key in DER. The endpoint must allow the anonymous user on its certificate-authenticated, encrypted session.

```sh
target/debug/cemsd capture-meter --contract contract.json \
  --endpoint opc.tcp://meter.example:4840 --server-cert server.der \
  --client-uri urn:plant:commissioner --client-cert client.der --client-key client-key.der \
  --samples 65 --period-seconds 15 > shift.ndjson
```

When a reading is excluded, the review identifies its NDJSON line, reason and available UTC source time. A status exclusion preserves the full OPC UA code. Select the quarter link to inspect the surrounding meter evidence.

Choose a new output filename. The session emits NDJSON once the requested reads succeed. Use an integer-preserving reader for UInt64 counters above 2⁵³. Polling an instantaneous counter rarely captures an exact quarter boundary. Use a source-timestamped, latched register and obtain the close window independently from the site metering system. `just opcua-meter-trial` runs the acquisition and review on a simulated meter.

## Rehearse an OpenADR event

```sh
cargo run -p cemsd --features openadr -- openadr
cargo run -p cemsd --features openadr -- openadr --cancel-before-dispatch --json
```

Each command starts a fresh, authenticated OpenADR 3.1 VTN on loopback. The simulated site reads its event and projects both battery start times against the requested 1,100 kW quarter-hour mean. At close, the site reports the original battery-idle meter’s 285 kWh to the VTN and reads the report back. Withdrawal clears the request before either dispatch. Add `--json` to either run for the numeric receipt.

The private program defines `EMS_QUARTER_MEAN_LIMIT` in `KW` over 15 minutes. Its meter report uses `USAGE` in `KWH` with `DIRECT_READ`. The optional feature uses the upstream wire model, timeline and in-memory VTN with a ten-second loopback HTTP client that disables proxies and redirects. [OpenADR crate](https://github.com/hupe1980/openadr/tree/v0.6.0)
