# Changelog

All notable changes to ems. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — before 1.0, a minor version may
break anything, and says so here.

## [0.10.0] — 2026-11-28

### Breaking changes

- Gateway and collector webhook keys must use `whsec_` followed by standard Base64 encoding 24–64 random bytes. Replace literal keys on both ends and remove `webhook_tolerance_s`. The Standard Webhooks SDK enforces a five-minute window.
- The fleet update schema requires a fresh development database and site enrolment.

### Added

- Household Setup commissions SunSpec model 203 meters over RS-485 and reads cumulative thermal Wh from a wired M-Bus heat meter. Setup and doctor flag silent, unexpected and unusable replies.
- Industrial sites can commission a pinned OPC UA import register and compare exact Wh against independent quarter-hour closes in JSON or offline HTML.
- The operator desk shows up to 90 days of import and export, gateway freshness and staged update results. Households can inspect missing days, peak readings and EV departure receipts.
- `hemsd winter-day` follows storage, heating and EV charging across a metered winter day. `house --keep-history` retains the gateway ledger and `simulate --site-from` reuses the installed inventory.
- Releases include gateway and tariff OCI images, architecture-specific conformance receipts, and ten downloadable CloudEvents schemas. `just interfaces` exercises REST, CloudEvents, signed webhooks and MCP.

### Changed

- Local power history uses Chronix 0.11 and reopens 0.10 directories in place. On Chronix 0.9, export historical readings before configuring a fresh series directory. Keep the separate redb ledger.
- The wired heat-meter driver uses mbus-kit 0.2.0. Gateway events identify their versioned payload schema in `dataschema`.

### Fixed

- Gateway sign-in restores the requested view. Losing access to a departure receipt preserves visible energy history.
- EV planning accounts for midnight departures and complete charging quarters. The gateway checks write acknowledgements, and the simulated outlet accounts for partial-interval charging.
- A full heat-pump request permits full output. The tank distinguishes unmet hot-water demand from standing losses.
- `hemsd simulate --json --store` keeps diagnostics on stderr. Enabling the example levy or Modul 3 tables preserves the feed-in rate and standing charge.

## [0.9.0] — 2026-11-24

### Added

- Households can sign in through OIDC to compare 7 or 30 calendar days of import, export and generation. The portal distinguishes missing reports from measured zeroes and shows the latest simulated EV departure receipt.
- Operators can search and filter the fleet desk, check gateway and feeder freshness independently and load one site's check-in history.
- The building simulator compares two OCPP charging stations with a KNX feeder meter and reserves heating capacity using BACnet/IP zone readings. Its replays cover station reconnects, sensor faults and incomplete tenant readings.
- `hemsd ev-departure` rehearses an OCPP charging interruption and reconciles outlet delivery with SunSpec whole-house import. `hemsd house --run --manage-storage` exercises battery setpoints, measured AC power and timed release.
- `cemsd reconcile --format html` produces an offline interval review with links to meter exceptions and explicit offsets for the repeated Berlin hour.

### Changed

- Household cold starts use a project-owned synthetic demand profile in place of the bundled BDEW H25 table.
- Day and Month show savings only for comparable service and expose comparison coverage. Daily bills require a unique priced reading for every quarter. Setup shows the signed unit price and UTC offset for the active contract.
- SunSpec meters of types 201–204 report import and export. The driver checks write acknowledgements and rediscovers inverter controls after reconnecting. Gateway readiness follows the Steuerbox connection.

## [0.8.0] — 2026-11-23

### Added

- Household Setup commissions a SunSpec connection meter, observed PV and storage, and a fixed or signed tariff. `hemsd doctor` reports device, pairing and history status.
- Household Plan previews goal costs, checks the quote on confirmation and recovers an interrupted save. Shadow mode compares measured bills and modelled savings at equal service. Replay includes heat missing at its deadline.
- Household power history shows quarter-hour ranges and sample counts. Exports flag missing device histories. Webhooks report grid limits, device loss and failsafe recovery.
- The optional Matter appliance reader observes power through a local Matter Server. `histd` uses per-site MeterStore market bindings for register ingestion, settlement and export.
- `bemsd` replays a charging reduction across two simulated OCPP stations and compares community electricity allocations across the repeated autumn hour.
- `cemsd` compares battery dispatch times, reconciles meter traces and optionally rehearses OpenADR requests and cancellations over loopback.
- The site offers solar EV charging and industrial peak simulations. Both use build-time results from the native Rust models. The shared industrial replay is available through the `ems-sim` crate’s `peak` feature.
- Linux release archives and dependency inventories are available for all three edge daemons.

### Changed — breaking

- Household history uses Chronix 0.10 integer sample counts. For a gateway with an existing 0.9 tier, export the history and start with a fresh Chronix directory. Keep the separate redb store.

- Local diagnostic power history starts a new trace in 0.8. Cached readings from earlier control ticks cannot supply fresh-observation counts. Export any older diagnostic history before updating.
- Historian register submissions carry a stable source revision per quarter. Identical retries return 204, conflicting retries return 409 atomically, and settlement returns the effective RFC 3339 cutoff for a repeatable register read.

- MiSpeL registers retain measured energy when wholesale or EEG support prices are missing. Settlement reports the exported quarter that needs a price.
- S2 battery discharge factors now run from maximum discharge at 0 to idle at 1. EV mode factors use the charging point’s advertised minimum and maximum power. A manager receives a terminal abort if its instruction conflicts with a saved household goal or the device cannot follow it.
- The home API and day history now use the same receipt with whole cents and watt-hours. Measured charges and modelled savings have separate totals.
- In the home API, a saved goal persists in the response when device telemetry is missing. Its feasibility assessment can be null.
- Environment variables take the `EMS_` prefix: `EMS_<SERVICE>_*` for every
  service (`EMS_HEMSD_*` included) and `EMS_TEST_POSTGRES` for the test suite.
  `HEMS_TOKEN`, `HEMS_STORE` and `HEMS_API_TOKEN` belong to hemsd and keep theirs.
- Releases are titled `ems <version>`.

## [0.7.0] — 2026-11-22

The project is **ems**: the workspace of grid rules, guard, planner, simulator
and protocols that **hems**, the household product, is built on — and that a
building product can be.

### Changed — breaking

- The crates are `ems-*`: `ems-core`, `ems-device`, `ems-drv`, `ems-events`,
  `ems-flex`, `ems-forecast`, `ems-grid`, `ems-optimizer`, `ems-realtime`,
  `ems-service`, `ems-sim` and `ems-tariff` replace their `hems-*` names, with
  no other change. The `hems-*` crates stay at 0.6.0.
- Event types are `de.ems.*` and an event's source is `ems://sites/<site>`.
- The repository is `hupe1980/ems` and the documentation
  `hupe1980.github.io/ems`. `hemsd` keeps its name.

### Security

- `url` 2.5.9 (RUSTSEC-2026-0592: a stack overflow in `Url::origin()` on
  nested `blob:` URLs).

## [0.6.0] — 2026-11-21

The box talks to other systems, lets a household price a goal before setting
it, and its battery loses what was measured.

### Added

- **Events out.** The box tells the systems a household names — Home
  Assistant, Node-RED, an installer — what it tells the household: a § 14a
  reduction begins or ends, failsafe, a goal at risk, a device not following,
  the closed day. CloudEvents 1.0, signed per Standard Webhooks with a `whsec_`
  secret shown once; retried with the same id for 24 hours; to private
  addresses, or public ones over HTTPS when the household says so.
  `GET/POST /v1/subscriptions`, `POST /v1/subscriptions/{id}/revoke|test`,
  `GET /v1/deliveries`, *Other systems* on Setup, a `webhooks` section in the
  contract, a Home Assistant automation in the docs. New catalogue types
  `de.hems.site.day.closed`, `de.hems.site.goal.at-risk`,
  `de.hems.device.deviation.detected`, `de.hems.test.ping`.
- `hemsd house --run`: the virtual house and the box that manages it in one
  command. The house answers for its day's prices and sky as `tariffd` and
  `forecastd` would, so the box plans.
- `GET /openapi.json`: the box serves its own contract.
- **What a goal costs, before it is set.** `POST /v1/goals/{asset}/quote`
  solves the plan in force again with the proposed goal and answers the
  verdict and `extra_eur`; the goal card shows it on release, and only *Set*
  commits.
- **The plan in sentences.** Under the Plan strip: each device's next run
  (*the car charges 21:00–01:00*) and the export-or-store decision with both
  prices (*a kilowatt-hour in the battery is worth 43 ct to you now; exported
  it earns 8 ct*), from `/v1/home`'s new `worth`.
- During a § 14a reduction, Now shows each device's share of the budget, and
  `/v1/home`'s `reduction.budget_kw` says what was shared.

### Changed — breaking

- Every refusal under `/v1/` is an RFC 9457 problem document
  (`application/problem+json`), with a `type` per status on the API page.
- `DELETE /v1/pairing/{ski}` answers `204`, or `404` for a control box that is
  not trusted; `DELETE /v1/overrides` answers `404` when none is in force.
- `/s2/{asset}` takes `GET` only.
- `hems-sim`'s `BatterySim` has a part-load converter (`no_load_loss`,
  `proportional_loss`, `cell_efficiency`) in place of `efficiency_charge` and
  `efficiency_discharge`.

### Changed

- A notification opens Plan for a goal at risk and Setup for failsafe.
- **The battery loses what was measured.** `hems-sim`'s converter follows
  HTW Berlin's published part-load curve (30 W no-load, 2 % proportional, cells
  √0.95, 8 W standby), and the planner — on the reference days and on the box —
  prices storage at a 400-W night discharge rather than the rated 95 %. Every
  reference day moved: the January day saves €1,10, €0,83 of it on the bill.

### Fixed

- A device whose driver refuses every command — a SunSpec battery — was
  still planned as a store.
- `hemsd house` ignored `SIGTERM`.

## [0.5.0] — 2026-11-21

Three sibling crates move, and one of them closes a defect on the § 14a link.

### Changed — breaking

- `hems-drv` re-exports `eebus` 0.13: `LimitWrite::watts` is a method, a
  limit and the failsafe value are the exact `ScaledNumber` the Steuerbox
  sent, and `require` returns a `Result`.

### Changed

- `eebus` 0.13.1: limits are written and read to three decimals rather than
  whole watts; a limit the virtual house's Steuerbox could not write is
  logged rather than recorded as sent.
- `metering` 0.26: MiSpeL's effect date, consent deadline and state-aid month
  come from `metering::eeg::mispel::geltung`, and a Pauschal year's limits
  from the figures `metering` applies.
- `chronix` 0.9: one time library (`jiff`); `chrono-tz` and RUSTSEC-2026-0555
  are gone from the tree.

### Fixed

- The box's SHIP session could lose an incoming SPINE datagram — possibly the
  Steuerbox's limit write — when its failsafe deadline fired while a reply
  waited on a peer that had stopped reading. `eebus` 0.13.1 makes `recv`
  cancel-safe, and hems requires it.

## [0.4.0] — 2026-11-20

The box speaks first: it notifies, hangs on a wall, hands over on paper, and
runs against a house on the real protocols without hardware.

### Added

- **A virtual house** — `hemsd house` (feature `house`): a SunSpec inverter,
  meter and battery over Modbus TCP from `tokio-modbus`'s server and a
  Steuerbox over EEBUS, on a reference day's weather and load, managed by an
  ordinary `hemsd run`. It writes the configuration to run against it.
- **The API as a contract** — `services/hemsd/openapi.json` (OpenAPI 3.1,
  generated from the handlers' types), `hemsd openapi`, and
  `cargo xtask check-api`, which fails when the API and the contract differ.
- **Notifications from the box** — Web Push to a home-screen page (Declarative
  Web Push on iOS 18.4+, a small service worker elsewhere) for four occasions:
  a § 14a reduction begins or ends, a goal turns *tight* or *impossible*, a
  device stops following for ten minutes, failsafe. One switch each, quiet
  hours, the sentence only, encrypted (RFC 8291) and signed (RFC 8292) by the
  box. `GET/POST/DELETE /v1/push`, `POST /v1/push/test`.
- **A hot-water goal** — *50 °C by 18:00*, answered like the car's, with the
  temperature that is reachable when the goal is not.
- **The month** — the box keeps its last ninety closed days in whole cents;
  `GET /v1/days`, `GET /v1/days.csv`, a month under the receipt whose total is
  the sum of its days. `hemsd demo --days N`.
- **A wall display** — `#glance`: the Now view for a tablet, screen kept awake,
  a dark face at night.
- **The handover sheet** — `GET /v1/handover` and `GET /v1/qr/{ski,fingerprint,signin}`:
  one A4 page with the SHIP pairing QR, every § 14a device with its group and
  minimum, and a QR code that signs a phone in. `POST /v1/token` issues a new
  access code and invalidates the old one.
- **A commissioning record** — `GET /v1/commissioning?reference=…`: what is
  paired and set up, as a JWS signed with the box's SHIP key and bound to an
  order's reference, for a field-service tool to scan
  (`GET /v1/qr/commissioning`) or fetch and verify.
- **Devices found on the network** — `POST /v1/discover`: SHIP nodes over mDNS
  and a SunSpec probe of named private addresses.
- **`GET /v1/trust`** — whether the box kept its identity over a restart, and
  whether its control box is paired, heard from, silent or in failsafe.
- The expected electricity bill of the plan, on the Plan view (`ahead` in
  `/v1/home`); a waiting device says until when and at what price.

### Changed — breaking

- Goals are set per device: `PUT/DELETE /v1/goals/{asset}` replaces
  `/v1/goals/car`. Goals survive a restart.
- `/v1/home` takes the household's credential only; `plan_cost_eur` is gone.
- `/v1/status` no longer carries `plan_baseline_eur`.
- `hems-tariff`: `Levies::household(year)` and `Levies::latest_published(year)`
  replace `household_2026()` and `Default`; `Tariff` carries `levies_by_year`;
  the § 19 levy's wire name is `paragraph_19_strom_nev`.
- `hemsd`: optional `[tariff.levies]` overrides a published figure from the
  supplier's price sheet.
- `/v1/home`'s `receipt` says whether the day was `priced`.

### Fixed

- A daemon serving plain HTTP stopped after its shutdown grace period —
  twenty seconds after starting.
- The SunSpec driver wrote the production ceiling three registers too low in
  model 123, onto the register that disconnects an inverter.
- A SunSpec model longer than 125 registers (the DER models) was requested in
  one read, cut short and never decoded; it is now read in pieces.
- The production ceiling ignored the device's `WMaxLimPct_SF` scale factor.
- A SunSpec battery was announced as controllable though standard SunSpec has
  no storage setpoint: every command was refused and logged each second. It is
  now shown as *not connected*, and a refusal is logged once.
- Once a Steuerbox's connection closed, nothing told the § 14a driver that
  time was passing: the box never reached failsafe and a limit never expired.
- **The levies were the 2024 figures under a 2026 name** — KWKG 0,28, § 19
  StromNEV 0,64, Offshore 0,65 ct/kWh, with no way to override them. They are
  now a dated table of the published figures for 2025 to 2027, each quarter
  hour is priced on its own year's, and a box in a year nobody has published
  yet says which year it is pricing on. Every euro a box reported for 2026 was
  1,38 ct/kWh net too low on imported energy; the reference days moved with it
  (the January day: bill €20,64, saved €1,12).
- The default Konzessionsabgabe was 1,66 ct/kWh, which is no band of § 2 KAV;
  it is 1,59.
- The plan priced the house without hems with a thermostat that switched once
  per quarter hour, which made the unmanaged day look several times dearer
  than the day report measured it. Both plan costs now also charge what the
  horizon borrowed from the battery and the tank.
- The receipt's lines did not add up to the cost of the day: what the day left
  in or took from the stores is now a line of its own.
- A running box printed €0.00 on a receipt it had no prices for; it now prints
  energy and the § 14a lines only.
- `hemsd demo` on a network address simulated a whole day before refusing.
- A lower-case fingerprint was dropped from the SHIP pairing QR.
- Signing out from the page did not end the session.

### Security

- An energy manager's credential could read `/v1/home`.
- Approving, refusing and forgetting a control box are logged with who did it.

### Known limitations

- A device whose driver refuses every command — a SunSpec battery — is shown
  as *not connected* but still planned as a store, so the plan prices energy
  that will not be there.
- On a running box the month and the receipt show energy and the § 14a lines,
  not euros: pricing a day against the same house without hems runs only in
  a replay.
- Notifications are tested against RFC 8291's own vector and a local endpoint,
  not yet against a browser vendor's push service.

## [0.3.0] — 2026-11-18

The first published release: the box can be looked at, paired, and tried without
hardware.

### Added

- **The household page**, served by `hemsd` itself in German and English: what
  the house draws and why in one sentence, the cheap hours ahead, the car, every
  device with its reason, the plan for two days, a receipt for the day against
  the same house without hems, and pairing a Steuerbox. Plain HTML, CSS and
  JavaScript compiled into the binary; it loads nothing from the internet.
- **`GET /v1/home`** — the page's one document, built by the same function for a
  running box and a replay.
- **Car goals** — `PUT /v1/goals/car` (`target_soc`, `by_minute`), answered
  *met / feasible / tight / impossible* at once from the planner's own session
  model; the planner plans to the goal.
- **Sign-in for browsers** — `POST /v1/session` exchanges the household's token
  for an `HttpOnly`, `SameSite=Strict` cookie; a manager's token cannot sign in.
- **HTTPS on the box** — on by default whenever `hemsd` listens off loopback, with
  a self-signed certificate it issues once and keeps; `[api] tls`,
  `[api] insecure_lan`. S2 managers connect over `wss://`.
- **`hemsd demo`** — a reference day simulated through the real guard, arbiter
  and planner, served minute by minute through the household page.
- **`xtask/media.mjs`** — screenshots and a captioned walkthrough film of the real
  page, through the Chrome on the machine.
- **A cold box no worse than none** — BDEW's H25 profile as the load prior a new
  box starts from, scaled by `[site] household_kwh_per_year` or its first day;
  the cold January reference day now saves €0,50 where it lost €1,75.
- **MiSpeL in force** — the Festlegung of 01.10.2026 (in effect 21.10.2026),
  its consent period and Pauschaloption gate, settled through
  `metering::eeg::mispel` and held to the regulator's worked examples.
- **`cargo xtask check-actions`** — every GitHub Action pinned by commit.
- `THREAT_MODEL.md` — the system-level threat model.

### Changed — breaking

- MSRV **1.99**.
- `hemsd` refuses to listen off loopback in plain HTTP unless
  `[api] insecure_lan = true`.
- `histd`: each `[mispel.site]` needs `assigned_from`; a mid-month switch of an
  existing plant is refused. `hems-grid::mispel::RuleSet` has only the adopted
  Festlegung, and `effective_from` is a date.
- `hems-grid::modul3::Modul3Calendar::new` is fallible; `Transcription::calendar`
  returns a `TranscriptionError`.
- A static § 42c key is applied as a six-place Aufteilungsfaktor, rounded down.
- `hems-realtime::GuardConfig` names its default `Verursachungsregel`
  (`SteuVeZuletzt`); `metering`'s § 14a presumption is named, not defaulted.
- Upgraded: `agentplane` 0.61, `metering` 0.25, `eebus` 0.11, `chronix` 0.8 (a
  new on-disk format: a 0.7 series directory is refused), `s2-kit` 0.4,
  `rmcp` 3.11, `toml` 1.

### Fixed

- The box kept its SHIP key but re-issued its certificate on every start, so its
  fingerprint changed with each reboot; the certificate is now kept.
- A manager's name could forge an audit line; names are restricted to letters,
  digits, `-`, `_` and `.`.

### Security

- An energy manager's credential could reach the household's own routes: it
  could approve its own control box in place of the network operator's, forget
  the real one, manage other managers and set the car goal. `/v1/pairing`,
  `/v1/managers`, `/v1/goals` and `/v1/series` now take the household's
  credential only.
- `rmcp` 3.11 (RUSTSEC-2026-0457, -0461) and `ctutils` 0.4.8 (RUSTSEC-2026-0385).
- Every override and S2 instruction is logged with the credential that made it.

[0.10.0]: https://github.com/hupe1980/ems/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/hupe1980/ems/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/hupe1980/ems/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/hupe1980/ems/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/hupe1980/ems/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/hupe1980/ems/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/hupe1980/ems/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/hupe1980/ems/releases/tag/v0.3.0
