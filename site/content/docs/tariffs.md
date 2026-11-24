+++
title = "Tariffs and prices"
description = "Activate a signed electricity contract and trace the prices used in plans and settlement."
weight = 7

[extra]
group = "Control and metering"
+++

A household plans against the gross price of importing energy and the revenue from exporting it in each quarter-hour. The same wholesale price can produce different retail prices at two addresses because their contracts and network charges differ. [Market data service](@/docs/services.md#tariffd-the-price-stack).

## Use a signed electricity contract

In **Setup → Electricity contract**, choose the supplier’s compact JWS file.
Compare the displayed signing-key fingerprint with a copy you received
independently from the supplier. Review the contract, effective time and gross
price, then accept. The gateway keeps the approved file across restarts.
A current contract takes effect from the next quarter hour. A future contract
starts at Berlin midnight on its effective date.

The day receipt names the contract applied to the measured intervals. If a
dynamic index is missing, Setup labels its price as estimated and the affected
interval stays unpriced on the receipt. When the priced year or validity ends,
Setup identifies the expired sheet and the configured tariff takes over.

For suppliers, sign the exact UTF-8 sheet as a compact JWS with protected
`alg: "Ed25519"`, `kid` and a public `jwk` containing `kty: "OKP"`,
`crv: "Ed25519"` and `x`. Provide the RFC 7638 SHA-256 key fingerprint with
the order. [JWS](https://www.rfc-editor.org/rfc/rfc7515) · [Ed25519 algorithm identifier](https://www.rfc-editor.org/rfc/rfc9864)

## Check a seller sheet locally

From the workspace root, calculate the gross import price at the start of a quarter hour:

```sh
cargo run -p hemsd -- price-sheet crates/ems-tariff/examples/preisblatt-dynamic.json \
  --at 2027-01-05T17:00:00+01:00 --index-ct=-2
```

The example returns `30.02489` ct/kWh. For a fixed contract, omit
`--index-ct`. For a dynamic contract, supply the net DE-LU index specified
by the sheet for that slot. The JSON result names any national levy filled
from the published table. This offline calculation leaves the gateway’s
active contract untouched. Add `--signed` for a compact JWS to verify its signature and print its key fingerprint.

For a signed example, use the synthetic seller that exercises the gateway’s own review and planner:

```sh
cargo run -p hemsd -- price-sheet services/hemsd/tests/fixtures/tariff/dynamic.jws \
  --signed --at 2027-01-05T16:45:00+01:00 --index-ct=-2
```

At 16:45 the result is **20.50489 ct/kWh**. At 17:00 it becomes **30.02489 ct/kWh**. The published index stays at −2. The network rate changes from 10 to 18 net ct/kWh and VAT applies to the difference. The signer is labelled “EMS synthetic seller”. Setup shows the full unit price and the Berlin UTC offset when you review the first eligible quarter.

## The bill is a stack

Every layer moves for its own reasons and on its own clock.

| Layer | Moves with | Where it comes from |
|---|---|---|
| **Energy** | the market, every 15 minutes | § 41a EnWG, the EPEX day-ahead auction |
| **Network charge** | the time of day, if Modul 3 is chosen | BK8-22/010-A and the operator's price sheet |
| **Levies and taxes** | the calendar year | StromStG, KWKG, § 19 StromNEV, Offshore-Netzumlage, Konzessionsabgabe |
| **Value added tax** | rarely, and on all of the above | 19 % |
| **Feed-in** | the support regime, and the sign of the spot price | EEG §§ 19, 51, 53 |

The layers stay **separate all the way through** and are summed only at the edge.
The optimiser wants one number per direction per slot; the household wants to see
the stack; and the [Modul advisor](#the-modul-advisor) has to take it apart again
to answer “would Modul 2 be cheaper for us?”. A design that adds them up early
can serve exactly one of those three.

<pre class="mermaid">
flowchart LR
  subgraph pub["published, fetched by tariffd"]
    E["day-ahead energy<br/>ct/kWh, per quarter hour"]
  end
  subgraph conf["configured from the price sheet"]
    N["network working price<br/>+ Modul 3 level"]
    L["levies and taxes"]
  end
  E --> S["SlotPrice"]
  N --> S
  L --> S
  S --> IMP["import_ct, gross<br/>what the household pays"]
  S --> EXP["export_ct<br/>what it earns — zero when<br/>the spot price was negative"]
  S --> SH["shared_import_ct<br/>a § 42c allocation"]
  IMP --> P["the planner"]
  EXP --> P
  SH --> P
</pre>

### Money is exact, control is not

Prices and costs are `rust_decimal::Decimal` in **cents per kilowatt-hour**.
Optimisation runs on `f64`, because a solver has no use for exact arithmetic —
but anything a household is shown or billed comes back through `Decimal`, and the
conversion happens **once**, at `SlotPrice::import_f64`. A settlement that went
through a `double` is one nobody can reproduce.

## Levies change on 1 January, and a plan runs across it

Three of the levies are set nationally and published every October for the
year that follows. They are a dated table in `ems_tariff::levies`, held to
the publication by a test:

| ct/kWh, net | 2025 | 2026 | 2027 |
|---|---|---|---|
| KWKG-Umlage | 0,277 | 0,446 | 0,451 |
| Aufschlag für besondere Netznutzung (§ 19 StromNEV), up to 1 GWh a year | 1,558 | 1,559 | 2,027 |
| Offshore-Netzumlage | 0,816 | 0,941 | 1,143 |
| Stromsteuer | 2,05 | 2,05 | 2,05 |

Each quarter hour is priced on the figures of **its own year**, so a plan made
on New Year's Eve prices 23:45 and 00:00 on two tables — 0,80 ct/kWh apart,
gross, from 2026 to 2027. In a year nobody has published yet, the box prices on
the last one that was and says so on the household page; it does not invent a
figure.

The Konzessionsabgabe is the municipality's: 1,32, 1,59, 1,99 or 2,39 ct/kWh
by its size (§ 2 KAV), 1,59 where nothing is configured. A supplier's price
sheet is the authority for one customer, and `[tariff.levies]` takes any of
the figures from it:

```toml
[tariff.levies]
konzessionsabgabe = 1.32
```

## Market sources and units

The tariff library parses ENTSO-E, SMARD, aWATTar, Tibber and Energy-Charts responses. The source determines the price basis. A gross consumer figure already includes charges that a wholesale price still needs. Keep the basis with each value to avoid applying levies twice.

Hourly observations expand to four equal quarter-hours. Source timestamps are resolved as instants so the repeated local hour in October can contain different prices on its two occurrences. A missing slot stays unknown.

## § 51 EEG: negative prices earn nothing

§ 51 Abs. 1 reduces the **anzulegender Wert** to zero in a quarter hour with a
negative spot price. Both remuneration schemes are computed from it — § 53 Abs. 1
for the Einspeisevergütung, Anlage 1 zu § 23a Nr. 1 for the Marktprämie — so the
rule zeroes the tariff *and* the premium.

The rule turns on the sign of the **market** price, not on what the household
pays, so the raw spot price is kept beside the retail one. A household in
Direktvermarktung priced at `spot + Prämie` that still books the premium in a
negative hour is being paid in exactly the hours the statute is paying it to
stop.

Whether the rule reaches a plant at all is a **date**, and it is deliberately not
the same date that lifts the § 9 EEG cap — see
[the grid rules](@/docs/grid-rules.md#ss-51-one-meter-two-rules-two-clocks).

## Modul 3: time-variable network charges

Use the local network operator’s effective calendar for the year and site. Modul 3 is used together with Modul 1 and an intelligent metering system. A cheap market interval can fall in an expensive network window. The planner compares the combined retail cost.

Configure windows under `[tariff.modul3]` and validate them with `hemsd run --check --config house.toml`. Windows follow local wall-clock time. The fleet catalogue serves curated operator calendars at `/v1/modul3/{netzbetreiber}`. [Network charge modules](@/docs/grid-rules.md#modul-3-time-variable-network-charges).

## The Modul advisor

The module comparison prices the household’s quarter-hourly history under the eligible arrangements. Its break-even output is annual consumption in kWh. A single winter day can illustrate which component changes without establishing what the household would save over a year.

## § 42c: the neighbours' roof is a cheap block, not a free one

A household in an energy-sharing community pays the community's **energy
component** for the kilowatt-hours the Aufteilungsschlüssel allocated, and its
supplier's for the rest. `SlotPrice::shared_import_ct` is the same stack with
the community's energy price substituted, so the two are directly comparable.

The result is less dramatic than the phrase “free solar from the neighbours”
suggests, and the arithmetic is the point: a community selling at **12 ct/kWh
net** still delivers a **34,0 ct** kilowatt-hour against the supplier's
**49,5** — a third off. Network charges, the Stromsteuer, the Konzessionsabgabe
and 19 % VAT do not care where the electron came from, because it crossed the
public grid to get here.

`SlotPrice::sharing_discount_f64` floors at **zero**, and never goes negative. That
keeps the planner's sharing term convex; the reasoning is in
[the planner](@/docs/optimizer.md#ss-42c-the-neighbours-roof-as-a-cheap-block-of-import).

## Two queries a household actually asks

Most of what a household wants from a price curve is not an optimisation.

```rust
let stack = PriceStack::resolve(&tariff, horizon);

// "When should the car charge?" — answerable without a solver.
let cheapest = stack.cheapest_slots(8);

// "When does feeding in earn nothing?" (§ 51 EEG)
let free = stack.zero_export_slots();

// The number that decides whether grid-charging a battery pays at all:
// the spread has to cover the round trip and the wear before anything is left.
let spread = stack.spread_ct();
```

## Where the prices come from at runtime

`tariffd` fetches the published curves, reconciles a curve that arrives twice
under a **written trust order**, and holds two days each way so a WAN outage
never costs a plan. That is on [the fleet page](@/docs/services.md#tariffd-the-price-stack);
what matters here is that none of it is in this crate, and that a price stack
which merely *stops short* of the horizon is still allowed — a horizon can
outrun the last published auction, and being indifferent about when to act out
there is exactly the state of knowledge.
