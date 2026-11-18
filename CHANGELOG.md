# Changelog

All notable changes to hems. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/) — before 1.0, a minor version may
break anything, and says so here.

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

[0.3.0]: https://github.com/hupe1980/hems/releases/tag/v0.3.0
