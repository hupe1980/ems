# Changelog

All notable changes to hems. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — before 1.0, a minor version may
break anything, and says so here.

## [Unreleased]

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

[Unreleased]: https://github.com/hupe1980/hems/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/hupe1980/hems/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/hupe1980/hems/releases/tag/v0.3.0
