+++
title = "The household page"
description = "What the box shows the people who live with it: one number, one sentence, the cheap hours, the car, a receipt — and why every device is doing what it does."
weight = 2

[extra]
group = "Start"
+++

The box serves its own page, on the household's network, from the same binary
that runs the guard. No app store, no account, no cloud: open the box's
address on a phone, sign in once with the code from the handover sheet, and
add it to the home screen.

<figure class="screens">
  <img src="../../screens/now-reduced.png" width="260" alt="Now: the network operator is reducing the connection and the battery carries the house" loading="lazy">
  <img src="../../screens/plan.png" width="260" alt="Plan: the next two days, cheap windows in green" loading="lazy">
  <img src="../../screens/day.png" width="260" alt="Day: the receipt" loading="lazy">
</figure>

<p><video src="../../media/walkthrough.mp4" controls muted playsinline preload="metadata" width="320"></video></p>

Every image and the film are the real page, rendering a simulated January day
through the real guard, arbiter and planner — `hemsd demo`, below.

## Four views, one idea each

| View | What it answers | How |
|---|---|---|
| **Now** | *What is the house doing, and why?* | One figure — what the house draws or feeds in — and one sentence. While the network operator reduces the connection, the sentence says so in the operator's colour and what the box is doing about it. Then the cheap hours of the next day, the car, and every device with its reason |
| **Plan** | *What will happen tonight?* | The next two days as one strip: bars for what the house will draw, green where power is cheap *for this household*. Touch a quarter hour and it explains itself, with what the same energy would have cost in the cheapest window |
| **Day** | *What did today cost?* | A receipt: the bill, battery wear and comfort priced in, the same house with the same battery and no hems, the saving — and how much of it reached the bill |
| **Setup** | *Is it paired?* | The SKI and certificate fingerprints in groups of four, to compare with a label by eye; a waiting Steuerbox to approve or refuse; the language |

## Every watt says why

A setpoint cannot be built without a reason (`Setpoint::new(value, reason)` is
the only constructor), so the page never guesses. The box sends a reason as a
key and its values — `plan.charge`, `price_ct = 36`, `among = 15` — and the page
owns the sentence, in German and English:

> *Läuft mit 3,3 kW — 36 ct gehört zu den günstigsten 15 % der nächsten zwei Tage.*

"Cheap" is measured against this household's own next two days, not against a
number: 36 ct is the cheapest hour of a winter week once network charges and
levies are in. A device is only said to run *because* it is cheap when it is
in the cheaper half of the plan; otherwise it runs because the plan needs the
energy then, and says that. A reason the page has no sentence for fails the
build rather than showing a household an identifier.

## A goal, answered before you leave

*Car at 80 % by 07:00* is a goal, not a boost button. The page answers it at
once, from the planner's own session model:

| Verdict | Means |
|---|---|
| **feasible** | the energy fits comfortably into the hours left |
| **tight** | it fits only if nothing else needs the connection — a § 14a reduction or a phase switch would cost it |
| **impossible** | not by then; the page says the level that *is* reachable rather than quietly planning for less |

The time is the wall clock's: on the night the clocks go forward, 07:00 is
still 07:00.

## When the network operator reduces the connection

A § 14a reduction is the one moment a household's devices are commanded by
somebody else, and the page leads with it: the operator's colour, the power
that stays guaranteed, and which devices share it. When the control box falls
silent and the stored failsafe value applies instead, the page says
*failsafe* — not *reduced* — because they are different situations with
different ends.

## Signing in, and the network it runs on

- **HTTPS on the household network, by default.** The box issues its own
  certificate on first start and keeps it, so its fingerprint — on the Setup
  view and the handover sheet — stays the same after a restart. Plain HTTP on a
  network address is refused unless `[api] insecure_lan = true` says so, for a
  box behind a proxy that terminates TLS itself.
- **One sign-in.** The access code is exchanged once for a cookie no script on
  the page can read (`HttpOnly`, `SameSite=Strict`). A link with the code in its
  fragment signs in by scanning, and the code is wiped from the address bar.
- **Nothing from anywhere else.** The page loads no fonts, scripts or images
  from the internet, so it works with the WAN down — which is when a household
  most wants to know what its box is doing.
- **An energy manager's credential does not open it.** A manager may drive
  devices; it may not read the household's life.

## Try it without a box

```console
$ cargo run -p hemsd -- demo                  # http://127.0.0.1:8080
$ cargo run -p hemsd -- demo --day summer     # or deadline, capped, offline…
```

`demo` simulates one reference day once, minute by minute, and serves the page
on it with a slider through the day. It listens on this machine only and asks
for no credential, because it carries no household and drives no device.

The screenshots and the film on this page come from it:

```console
$ node xtask/media.mjs screenshots http://127.0.0.1:8080 site/static/screens
$ node xtask/media.mjs walkthrough http://127.0.0.1:8080 site/static/media/walkthrough.mp4
```

Both drive the Chrome already on the machine over its DevTools protocol, with
the page in its capture mode — no demo controls, exactly what a household sees —
and both fail if any view scrolls sideways at phone width.
