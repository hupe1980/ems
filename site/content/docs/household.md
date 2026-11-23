+++
title = "Using the household gateway"
description = "Read live power, set charging and hot-water goals, inspect receipts and commission the gateway."
weight = 2

[extra]
group = "Start"
+++

Try the [solar charging simulator](@/solar-charging.md) to see when a wallbox can use the rooftop surplus on one or three phases.

Open the gateway’s HTTPS address on the household network and enter the access code from the handover sheet. Add the page to your home screen for quick access to Now, Plan, Day and Setup.

<figure class="screens">
  <img src="../../screens/now-reduced.png" width="260" alt="Now: the network operator is reducing the connection and the battery carries the house" loading="lazy">
  <img src="../../screens/plan.png" width="260" alt="Plan: the next two days, cheap windows in green" loading="lazy">
  <img src="../../screens/day.png" width="260" alt="Day: the receipt" loading="lazy">
</figure>

<video src="../../media/walkthrough.mp4" controls muted playsinline preload="metadata" width="320" aria-label="Walkthrough of Now, Plan, Day and Setup on a simulated winter day"></video>

Every image and the film are the real page, rendering a simulated January day
through the real guard, arbiter and planner — `hemsd demo`, below.

## Four views, one idea each

| View | What it answers | How |
|---|---|---|
| **Now** | What is the house doing? | Grid import or export, device readings and reasons, and any active grid limit. |
| **Plan** | What will run tonight? | A two-day strip, the first device runs, the export-or-store decision and goal quotes. After a failed update, the page dates the retained plan. |
| **Day** | What did it cost? | A receipt and monthly history. A priced day shows the bill, wear, comfort and savings. A metered day shows energy, grid limits and charges for fully priced intervals. |
| **Setup** | Is everything working? | Device health, Steuerbox pairing, discovery, notifications and the handover sheet. Open the operator details for certificate fingerprints. |

## Every watt says why

Select a device on **Now** to see its current reading and the reason for its command. Check whether a grid limit, household override, external manager, active plan or fallback is in effect. A requested ceiling and an observed power reading can differ while a device responds.

## A goal, answered before you leave

*Car at 80 % by 07:00* and *hot water at 50 °C by 18:00* are goals, not boost
buttons. The page answers each at once, from the planner's own model of the
device:

| Verdict | Means |
|---|---|
| **feasible** | the energy fits comfortably into the hours left |
| **tight** | it fits only if nothing else needs the connection — a § 14a reduction or a phase switch would cost it |
| **impossible** | not by then; the page says the level or temperature that *is* reachable rather than quietly planning for less |

When you adjust a saved target at the same deadline, the slider shows an
approximate cost if the current plan prices that device. Releasing the slider
asks the box what the plan would cost with the new goal: *feasible — 42 ct
more than now*. The box solves the same plan it is following with the goal
changed and subtracts — energy, battery wear and what it borrows from the
stores — so the price is the plan's own, to the cent. Nothing changes until
*Set*.

For the car, enter an optional cap in whole ct/kWh. Only quarters with a confirmed gross import price at or below the cap count toward the departure goal. The cap also pauses charging from local surplus during higher-priced or unpriced quarters. Clear the field and confirm to remove it.

Saving a goal triggers a new plan. Once that plan is ready, the gateway gives
the household’s schedule priority over conflicting instructions from a connected
energy manager. Grid and equipment limits still apply.

To bring charging forward, open **Preview charging now** on a saved car goal. The box compares the earliest attainable charge with the planned departure. Confirm the displayed difference to start. Charging returns to the plan at the goal or deadline, on unplug or stale vehicle data, or when the confirmed tariff no longer permits it. **End immediate charging** returns to the plan immediately.

The time is the wall clock's: on the night the clocks go forward, 07:00 is
still 07:00. A goal survives a restart. A device that waits says until when,
and at what price: *Waiting until 21:00 — 31 ct then.*

## When the network operator reduces the connection

During a reduction, **Now** shows the grid limit and which devices share it. If the control box stops sending heartbeats and the stored failsafe limit takes effect, the page shows *failsafe*. Open **Setup** to check when the last heartbeat was accepted.

## The box speaks first

In **Setup**, enable notifications for the events you need and grant permission in a browser that supports Web Push. On iPhone, add the gateway to the Home Screen first. Delivery depends on a working push subscription and a reachable push service.

| Occasion | Sent | A tap opens |
|---|---|---|
| The network operator reduces the connection | when it begins and when it ends | Now |
| A goal is at risk | when its verdict turns *tight* or *impossible* | Plan, where the goal is |
| A device stops following | after ten minutes, once | Now |
| Failsafe | when the control box falls silent | Setup, whose trust lines say what the control box last did |

Each is a switch on the Setup view, with quiet hours from 22:00 to 07:00. A
notification carries the sentence the page would show and nothing measured;
it is encrypted for the one browser that subscribed, signed by the box, and
sent only to the browser vendors' push services. A box with no internet
sends nothing and queues nothing — a reduction that ended an hour ago is not
news. On a reference day the box speaks at most six times.

While a reduction is in force, Now shows how the box shares what the network
operator left: a bar per device, by need — *car 6.2 kW · heat pump 4.3 kW*.
§ 14a limits the draw from the grid, so what the roof and the battery cover is
added to it; a device that only measures is given no share.

**Other systems** hear the same events with their numbers: on Setup, a
household adds Home Assistant, Node-RED or its installer by URL, chooses
*reductions and failsafe* or *everything*, and copies the secret the box shows
once. Each event is a signed CloudEvent; [the API](@/docs/api.md#events-out)
has the details and a Home Assistant automation.

## Observe a household

For a meter-only installation, first confirm which independent controller
enforces the site’s grid limits. Record its name at the top of the gateway
configuration, before any TOML table.

```toml
mode = "shadow"
external_limit_controller = "Site PLC A"
```

Configure the connection meter. Shadow mode accepts SunSpec, register-map
and EEBUS EV telemetry and requires the SHIP listener and S2 control to be
disabled. Run `hemsd run --check --config <file>` before startup. The page
shows **Observing**, Plan previews decisions, and each receipt counts its
observed quarter hours. When ready, authorize a handover in Setup. The
installer has one hour to coordinate release of the independent controller,
commission the gateway’s grid-limit inputs, remove `external_limit_controller`
and restart in managed mode with the same site and store. You can revoke
authorization in Setup while the box observes. Setup identifies missing meter
readings or actuator control evidence. Handover is available for a metered
fixed-load site with optional confirmed SunSpec model 123 inverter.

## The month

The box keeps its last ninety closed days. Under the receipt, a month: one
bar per day, red and downward where a day lost, a gap where the box was off,
and a total that is the sum of the receipts to the cent. Touch a day for its
receipt or download the days as CSV. A closed receipt keeps the figures it had
when the day ended. A fully measured day with known rates shows import charges
less export revenue and its share of annual fixed charges. A partial bill shows
how many quarter hours had both a reading and a price. Day and Plan count the first
fourteen scored days. Open a scored receipt to see how often solar production
and household load fell within the forecast range.

On a fixed-load site with an optional controllable PV inverter reporting
available power, a complete day can also compare grid costs at equal household
demand. A stationary battery can qualify when both modelled AC flows match the
measured storage flow and the plan at each reading. A charge point qualifies
when it continuously reports a disconnected car and zero power. Each arm uses the same inverter model and the same sunshine. Grey bars
mean the difference falls within the model’s measured error. Red bars show
losses. A separate battery margin shows grid charging and the import it
displaced after storage losses, for cycles returning to their initial measured
charge. It is never added to the comparison total.

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

## Check a running gateway

Setup lists devices that need attention, including the connection meter. On the gateway terminal, `hemsd doctor --config house.toml` reads the same device checks and tests pairing, the control clock and the connection. Supply the household credential through `HEMS_TOKEN` or `[api].token`. Use `--ca-cert hems.pem` for the gateway’s self-signed HTTPS certificate.

**Meter history** appears in Setup if an older quarter awaits recalculation after a late observation. Check again after automatic maintenance. A busy history reader has a separate retry message. If inspection fails, check the gateway log. Recent quarters can use live observations while older ones are stored; this is normal.

## The handover sheet

Setup shows which devices have reported a measurement. Open the operator details for the SKI and certificate fingerprint.

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

On a new gateway, run `hemsd run --config house.toml` and open the printed
HTTPS address. Enter the gateway code, three-phase fuse rating, connection
meter address and unit, and fixed tariff from the contract. Add an optional
support contact for device faults. Optionally select **Observe solar PV** or
**Observe battery** and enter the commissioning dates and nameplate values.
These options use shadow mode and require an independent site controller.
A verified inverter with active-power limiting can participate in a bill
comparison when it also supplies the required telemetry. The battery reports
state of charge.
Match each reported device to its label and review the TOML. Setup confirms
installation once the meter and every selected device supply a measurement.

The same view finds devices: SHIP nodes announcing themselves on the
network, and SunSpec devices at the addresses an installer types in — each
listed as configured, found, or configured and not answering. Select the Modbus
unit and port when a gateway fronts several devices. For a supported inverter
with immediate controls, open **Review PV inverter**. Check the address, unit
and proposed driver block against the inverter’s label. Setup validates the
block with your site’s AC rating. Close the review to return to discovery, or choose **Apply and restart gateway**
to activate it. Setup shows the result after reconnection. A changed or
unreachable inverter, startup failure or missing first PV measurement restores
the previous configuration. Keep the access code and gateway identity across
restarts with a persistent store. A read-only deployment can still review
the exact change.

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



## Confirming a goal

Plan keeps the saved target visible while you change the charge or hot-water
goal. The confirmation names the proposed target and deadline. If the displayed
quote changes when you confirm, review the difference and confirm again.
Discard returns to the saved goal. After an interrupted save the page reads
the stored goal before offering another write. On a box without a usable plan
you can explicitly set a goal with its cost marked unavailable.

## Matter appliance power

Build with `cargo build -p hemsd --features matter`. Pair the appliance in your local Matter Server, select an endpoint exposing Electrical Power Measurement, and add this section to the household configuration:

```toml
[matter]
name = "Washing machine"
fabric = "beef" # replace with the compressed fabric ID in hex
node = "a"      # hexadecimal node ID (10 in decimal)
endpoint = 2    # decimal endpoint ID
port = 5580
```

Use the controller’s inventory for the node and endpoint. In the Matter Server dashboard, open the server details and copy **Compressed FabricId**. Convert that decimal number to hex for the `fabric` setting, for example `python3 -c 'print(format(48879, "x"))'` prints `beef`. Keep the commissioner and its fabric storage on the gateway host. The reader connects to its IPv4 loopback WebSocket API and requires a server whose supported range includes schema 11.

Check the configuration offline with `hemsd run --check --config PATH`. Before starting household control, read the selected appliance once:

```sh
hemsd matter-read --config PATH
```

Success prints one JSON observation. A numeric `watts` is a measurement, and `null` with state `unknown` means the appliance responded without a value. A failed read exits nonzero and writes a diagnostic to stderr. Check the commissioner address, compressed fabric ID, node and endpoint indicated by the diagnostic. Each attempt has an eight-second deadline. To exercise loss and recovery without an appliance, run `just matter-recovery` from the workspace root.

Start the gateway with `hemsd run --config PATH`. Open **Setup → Matter appliance**. Every 15 seconds the gateway requests active power. Zero means the device reported zero. Unknown means it returned null. An unavailable device keeps its last successful reading labelled stale. If the browser loses contact with the gateway, Setup marks its cached reading stale until contact resumes. The timestamp is when the gateway received the device read. Negative watts indicate generation. The reading is diagnostic and carries no control authority.


## Inspect a quarter hour

On a gateway with `[series]` configured, open **Day → Grid connection**. Choose a Berlin date and quarter to inspect a short-lived spike, a measured zero or an interval with no readings. On the autumn clock change the UTC offset distinguishes the two occurrences of the same local time.

When upgrading an existing Chronix 0.9 gateway, download the history with the old gateway first. Configure a fresh `[series]` directory for the integer-count tier. Keep the old directory for reference and retain the gateway’s redb store.

The mean and range describe the readings received. A single observation can establish a peak but cannot establish how long it lasted. The local JSON download covers 1,095 days of quarter-hour summaries. The latest quarter may still be in progress. `complete` means every requested point was retrieved. A partial file lists the failed points and keeps the successful reads. [History API](@/docs/api.md#local-power-history)
