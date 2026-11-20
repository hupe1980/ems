+++
title = "The household page"
description = "What the box shows the people who live with it: one number, one sentence, the cheap hours, goals with an answer, a receipt that adds up to a month — and the four things it will interrupt you for."
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
| **Plan** | *What will happen tonight?* | The next two days as one strip: bars for what the house will draw, green where power is cheap *for this household*, and the electricity bill the plan expects. Touch a quarter hour and it explains itself, with what the same energy would have cost in the cheapest window. Below it, the goals |
| **Day** | *What did it cost?* | A receipt: the bill, battery wear and comfort priced in, the same house with the same battery and no hems, the saving — and how much of it reached the bill. Under it, the month |
| **Setup** | *Is it paired?* | The SKI and certificate fingerprints in groups of four, to compare with a label by eye; a waiting Steuerbox to approve or refuse; whether the box kept its identity over the last restart; devices found on the network; notifications; the handover sheet |

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

*Car at 80 % by 07:00* and *hot water at 50 °C by 18:00* are goals, not boost
buttons. The page answers each at once, from the planner's own model of the
device:

| Verdict | Means |
|---|---|
| **feasible** | the energy fits comfortably into the hours left |
| **tight** | it fits only if nothing else needs the connection — a § 14a reduction or a phase switch would cost it |
| **impossible** | not by then; the page says the level or temperature that *is* reachable rather than quietly planning for less |

The time is the wall clock's: on the night the clocks go forward, 07:00 is
still 07:00. A goal survives a restart. A device that waits says until when,
and at what price: *Waiting until 21:00 — 31 ct then.*

## When the network operator reduces the connection

A § 14a reduction is the one moment a household's devices are commanded by
somebody else, and the page leads with it: the operator's colour, the power
that stays guaranteed, and which devices share it. When the control box falls
silent and the stored failsafe value applies instead, the page says
*failsafe* — not *reduced* — because they are different situations with
different ends.

## The box speaks first

A household that has to open a page to learn its car will not be ready has
been told too late. Added to a home screen, the page can be sent
notifications — on an iPhone from iOS 18.4, on every other current browser —
for four occasions and no others:

| Occasion | Sent |
|---|---|
| The network operator reduces the connection | when it begins and when it ends |
| A goal is at risk | when its verdict turns *tight* or *impossible* |
| A device stops following | after ten minutes, once |
| Failsafe | when the control box falls silent |

Each is a switch on the Setup view, with quiet hours from 22:00 to 07:00. A
notification carries the sentence the page would show and nothing measured;
it is encrypted for the one browser that subscribed, signed by the box, and
sent only to the browser vendors' push services. A box with no internet
sends nothing and queues nothing — a reduction that ended an hour ago is not
news.

## The month

The box keeps its last ninety closed days. Under the receipt, a month: one
bar per day, red and downward where a day lost, a gap where the box was off,
and a total that is the sum of the receipts to the cent. Touch a day for its
receipt; download the days as CSV.

<figure class="screens">
  <img src="../../screens/month.png" width="260" alt="Day: the receipt and, under it, the month — a bar per day and the total saved" loading="lazy">
  <img src="../../screens/goals.png" width="260" alt="Plan: goals for the car and the hot water" loading="lazy">
  <img src="../../screens/record.png" width="260" alt="The completion code for an installer's order system, bound to an order number" loading="lazy">
</figure>

## A display for the wall

`#glance` is the Now view for a tablet in a stand: the figure readable from
across a room, the sentence, the cheap hours and the car — no controls, the
screen kept awake, a dark face at night, and the operator's colour around
the edge while a reduction is in force. A tap leads back.

<figure class="screens wide">
  <img src="../../screens/glance-reduced.png" width="560" alt="The wall display during a reduction by the network operator" loading="lazy">
  <img src="../../screens/glance-night.png" width="560" alt="The wall display at night" loading="lazy">
</figure>

## The handover sheet

One A4 page from the Setup view: the SKI and the certificate fingerprint as
text and as QR codes, every controllable device with its § 14a group, its
power and the minimum it is owed, and a QR code that signs a phone in. The
sheet is a key and says so; *Issue a new access code* on the Setup view
makes a lost one worthless.

For whoever ordered the installation there is a **completion code**, on a
view of its own so it is never on screen beside the household's sign-in
code: the box states, under the signature of the key the network operator
paired with, which control box is paired and which devices are set up, bound
to the order number. An order system can open it with the number filled in —
`https://<box>/#record=<order number>` — so nobody types one at the wall. The
record is a download first and a QR code as the shortcut; anyone holding the
box's SKI can [verify it](@/docs/api.md).

The same view finds devices: SHIP nodes announcing themselves on the
network, and SunSpec devices at the addresses an installer types in — each
listed as configured, found, or configured and not answering.

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
$ cargo run -p hemsd -- demo --days 30        # with a month of days before it
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
