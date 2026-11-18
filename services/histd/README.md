# histd

The fleet's record of what every box did, for
[hems](https://github.com/hupe1980/hems).

Two records with two different owners and two different reasons to exist, and
keeping them apart is most of the design.

| Record | Whose question | What it is |
|---|---|---|
| **Evidence** | the network operator's | `[A1 7.2]` says what a § 14a control event is documented with — the ceiling, when it arrived, what was done about it, a trace of what the connection point drew — and `[A1 7.3]` says it is kept for **two years** |
| **Settlement** | the household's invoice | the quarter-hour meter registers MiSpeL's Abgrenzung and § 42c's allocation are computed from |

Four things fall out of that split:

- **Retention is a column**, so “what will you still have in eighteen months” is a
  query rather than an argument.
- **Quantities are exact `NUMERIC`**, never floats. A settlement that went
  through a `double` is a settlement nobody can reproduce — and because the
  database has an exact type, `SUM(grid_draw_kwh)` over a settlement year is a
  query rather than a loop over 35 040 strings.
- **A correction is a version, not an overwrite.** A register is restated after
  the month has been settled and the Nachweis sent, so `quarter_hour` is
  append-only and keyed by the instant a value was learned: an ordinary read
  returns the current answer, and `?as_of=` returns the one the document was
  computed from.
- **A fleet's writes do not queue behind one lock.** A box's evidence write is
  on the path a network operator's Nachweis is built from, and it does not wait
  behind eight household exports.

## Three exports, authorised differently

| | |
|---|---|
| `POST /v1/sites/{site}/quarter-hours` | a box writes its own registers |
| `POST /v1/sites/{site}/events` | a box writes its own control events |
| `GET /v1/sites/{site}/nachweis` | the **network operator's** Nachweis |
| `GET /v1/sites/{site}/mispel?year=&month=&as_of=` | the MiSpeL settlement, `[MiSpeL A1 4.2]` / `[A2 4.2]` — `as_of` settles from the registers as they stood at an instant, which is how a document already handed over is reproduced |
| `GET /v1/sites/{site}/export` | the **household's** Data Act Article 4 export |

A box's credential reaches its own site and no other. An operator's reaches every
household's § 14a evidence and **none** of their Data Act exports: a Nachweis is
the record of what the operator itself commanded and what the connection point
drew, and it is theirs to check — while Article 4 is a right of the *user*, and a
fleet token is not a household.

The **MiSpeL settlement** sits with the export rather than with the Nachweis. It
is the household's own levy privilege (§ 21 EnFG, under the MiSpeL Festlegung in
effect since 21.10.2026) and the flows
behind it — how full the store was, when it was charged from the grid, what the
roof earned — and the Festlegung has the *Anlagenbetreiber* produce and submit
it. So the same credential that cannot read one cannot read the other, and it is
off the `/mcp` surface for the same reason.

Three things about it are the design rather than the plumbing. The **option is
declared per site** (`[mispel.haus-1]`) and an undeclared site is refused, because
Ausschließlichkeit, Abgrenzung and Pauschal each produce a *different* Nachweis
from the same registers and a guessed one is arithmetically perfect and about
somebody else's installation. Ausschließlichkeit settles nothing and is still
**checked** against those registers: it is a claim that no quarter hour shows
grid draw and storage charging at once, `(1)¼` is what measures that, and the
export reports the figure and names any quarter hour that breaks it. The
**period comes from the calendar and the rules**, not from the caller: a year
and a month are narrowed to what the Festlegung governs for *this* site — not
before 21.10.2026, not before the site's `assigned_from` (the first of a month
for an existing plant, which `histd` checks at start-up), not before October
2027 without the operators' consent, and for the Pauschaloption not before the
month after EU state-aid approval (`mispel_state_aid_approved_on`). What starts
mid-period settles as a Rumpfmonat or Rumpfjahr, and the document prints the
`window` and the `conditions` it rests on; a refused period answers `400` with
the clause in the body. Boundaries come from the Berlin calendar, so March and
October have the hours they really have. And the **denominator is on the document** —
`quarter_hours_expected`, `quarter_hours_present`, `complete` — because a quarter
hour the box could not price gets no register, so a month legitimately has gaps
and a settlement over part of one under-reports every quantity in it while
looking whole.

All four **Basisfälle** settle. A4 is the installation whose storage system has a
meter of its own — `(17)A4` charges the round-trip loss to the store rather than
to the household — and it needs `Z3V¼`/`Z3E¼`, which are nullable here. The null
means *not separately metered* rather than zero: a household declared A4 whose
store went unread is refused rather than handed a settlement claiming its battery
stood still.

## Where this runs, and why it is not on the box

The edge is a **single** daemon, so a gateway runs `hemsd` and nothing else, and
the box's own copy of these records lives in *its* embedded stores behind a
store-and-forward outbox. This daemon is the **fleet** side: everybody's two
years, queryable, which is what a Nachweis and a Data Act export are asked for at
scale.

## PostgreSQL

Size is not why a fleet service needs a server. **One writer** would put every
household's forwarded evidence behind one mutex; **one node** could not be
replicated, deployed without downtime, or outlive its machine; and **no exact
decimal** would make every settlement quantity a string to parse back.

The box is the other case, and an embedded store is right there: one process,
one writer, no network, offline-first, and a file an installer can copy off a
failed unit.

The tests run against PostgreSQL because the daemon does — a query checked on a
different engine is checked by nothing. A container runtime is a prerequisite;
`HEMS_TEST_POSTGRES` points the suite at a server that is already running
instead. There is deliberately no way to skip.

## License

MIT OR Apache-2.0
