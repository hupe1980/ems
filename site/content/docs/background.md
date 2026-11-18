+++
title = "The German grid in ten minutes"
description = "What § 14a EnWG, a Steuerbox, Modul 1–3, § 9 EEG, MiSpeL, § 42c and EEBUS are — the background every other page assumes, with a glossary."
weight = 3

[extra]
group = "Start"
+++

A German household with a roof, a battery, a heat pump and a car is no longer
just a consumer. Since 2024 a stack of rules has made it a small, regulated
power station. Each rule is simple on its own; the hard part is that they all act
on **one connection point** at once, and some of them pull in opposite
directions. This page explains each rule in a paragraph and says which part of
hems implements it.

> [!NOTE]
> This is orientation, not legal advice. Each section links to the page where
> the rule is implemented, and that page cites the clause every number comes
> from.

## Who is who

| Role | German | What they do here |
|---|---|---|
| Network operator | Verteilnetzbetreiber (VNB) | Runs the local low-voltage grid. May reduce controllable devices under § 14a and owes the household a lower network fee in return |
| Metering point operator | Messstellenbetreiber (MSB) | Installs and runs the meter and, where there is one, the smart meter gateway and the Steuerbox |
| Supplier | Lieferant | Sells the electricity. Must offer a dynamic tariff (§ 41a EnWG) |
| Regulator | Bundesnetzagentur (BNetzA) | Writes the Festlegungen — binding determinations such as BK6-22-300 (§ 14a) and MiSpeL |
| Energy manager | Energiemanagementsystem (EMS) | Decides what the devices behind the connection do. **This is hems** |

## § 14a EnWG — the network operator may turn devices down

A **steuerbare Verbrauchseinrichtung (SteuVE)** is a controllable consumer above
4,2 kW: a private wallbox, a heat pump, an air conditioner, or a battery while it
charges. Since 1 January 2024 the network operator may limit them when the local
grid is congested, and in return the household pays a lower network fee.

The limit is never zero. The operator must leave a **minimum power**: 4,2 kW for
a single device, more for several, and more for a large heat pump. Where an
energy manager is present, the limit applies to the **sum** the controllable
devices draw from the grid, not to each device. That is why a battery
discharging into the wallbox during a reduction is lawful and worth money.

**In hems:** the guard enforces the limit as a hard bound that nothing above it
can widen, and the box keeps the evidence of every reduction for two years.
[The grid rules → § 14a](@/docs/grid-rules.md#ss-14a-enwg-which-devices-and-how-far-down)

## The Steuerbox and EEBUS — how the limit arrives

The **Steuerbox** (FNN control box) is a small device the metering point
operator installs at the connection, behind the smart meter gateway. It receives
the network operator's instruction and passes it on, either to relay contacts or,
digitally, over **EEBUS** to an energy manager.

**EEBUS** is two layers: **SHIP** is the secure transport (TLS with mutual
authentication; each device is identified by its **SKI**, a hash of its public
key), and **SPINE** is the data model. The use cases that matter here are **LPC**
(*Limitation of Power Consumption*: the § 14a limit, with a failsafe value if the
link goes quiet), **LPP** (the same for production) and **MGCP** (*Monitoring of
the Grid Connection Point*: what the meter measures).

**Pairing** means each side trusts the other's SKI. An installer reads the box's
SKI — hems prints it at start-up — and enters it into the operator's system.

**In hems:** the box is an EEBUS *Controllable System* over a real SHIP session.
[Devices → EEBUS](@/docs/devices.md#eebus-the-ss-14a-side)

## Modul 1, 2 and 3 — what the household gets in return

A household with a SteuVE chooses how its network fee is reduced:

| Module | What it is | Needs |
|---|---|---|
| **Modul 1** | A fixed annual discount on the network fee | Nothing extra |
| **Modul 2** | 60 % off the network fee's energy price for the controllable devices | A separate meter for them |
| **Modul 3** | Modul 1 plus a network fee that varies by time of day — cheap (NT), standard (ST) and high (HT) windows set by each network operator | An intelligent metering system (iMSys) |

Which one pays depends on how much the devices consume and when, so it is a
calculation rather than a preference.

**In hems:** the Modul advisor prices a household's own history under all three
and reports the annual consumption above which Modul 2 is cheaper, and the box
refuses to start on a Modul 3 calendar that breaks the BDEW rules. [Tariffs → Modul 3](@/docs/tariffs.md#modul-3-time-variable-network-charges)

## § 9 and § 51 EEG — the solar peak rules

Since the *Solarspitzengesetz* (February 2025), a new photovoltaic system may
feed at most **60 % of its installed power** into the grid until an intelligent
meter and a control device are in operation. Energy used in the house does not
count against the cap, so a battery or a car charging at noon recovers what the
cap would have cut off.

For new systems **§ 51 EEG** pays nothing for electricity fed in while the spot
price is negative. Negative-price hours are typically sunny weekend middays, so
self-consumption is worth most exactly when feeding in earns nothing.

**In hems:** both caps are measured at the connection point, and the planner
prices a negative-price quarter hour as what it is.
[The grid rules → § 9 EEG](@/docs/grid-rules.md#ss-9-eeg-two-caps-with-different-references)

## § 41a EnWG — prices that change every quarter hour

Every supplier must offer a **dynamic tariff** that follows the day-ahead
wholesale auction. The auction clears at noon for the next day in quarter-hour
products, so an energy manager knows tomorrow's prices a day ahead. The final
price per kWh is a stack: the wholesale price plus network fees, levies,
taxes and the supplier's margin. Only the first layer moves.

**In hems:** `tariffd` fetches the auction results from five public sources, and
the box plans 192 quarter hours (two days) ahead.
[Tariffs and prices](@/docs/tariffs.md)

## MiSpeL — which kilowatt-hour in the battery was green

A home battery can charge from the roof (green) and from the grid (grey). Before
2026, letting it do both cost a household its feed-in tariff for the stored
energy. The Bundesnetzagentur's **MiSpeL** determination (decided 1 October
2026, with operators given until 30 September 2027) allows mixed operation if the
flows are separated quarter hour by quarter hour: either by a formula over
measured flows (**Abgrenzungsoption**) or, for roofs up to 30 kWp, by a flat rule
(**Pauschaloption**).

**In hems:** the flow bookkeeping is implemented with the determination's own
numbered equations, in exact decimals.
[The grid rules → MiSpeL](@/docs/grid-rules.md#mispel-which-kilowatt-hour-was-green)

## § 42c EnWG — sharing with the neighbours

Since 1 June 2026 households in the same network area may share electricity
from a renewable plant over the public grid (**Energy Sharing**). The shared
energy is allocated quarter hour by quarter hour from the meters. For the
household it behaves like a cheap block of energy that is only available when the
neighbours' roof is producing.

**In hems:** the allocation is computed from the day's registers, and the planner
treats the share as a price band. [Tariffs → § 42c](@/docs/tariffs.md#ss-42c-the-neighbours-roof-is-a-cheap-block-not-a-free-one)

## S2 and SunSpec — talking to devices and to other managers

**S2** (EN 50491-12-2) is a European standard for describing what a device can
do rather than what it is: a battery is a store with limits, a dishwasher is a
programme that can start late. An external **Customer Energy Manager**, such as an
aggregator's, can drive devices through it.

**SunSpec over Modbus TCP** is the register map most inverters, meters and
batteries already speak. It needs no membership or certificate, which is why it
is where a real installation starts. It also has no authentication, so those
devices belong on a network segment only the gateway can reach.

**In hems:** S2 is the internal flexibility model, and the box serves it to an
external manager. SunSpec is the default device driver.
[Flexibility](@/docs/flexibility.md) · [Devices](@/docs/devices.md)

## The EU Data Act — the household's data is the household's

Since 12 September 2026 connected products must give their users access to the
data they generate. For an energy manager that data includes the measured series,
and it reveals when someone showered, cooked or was away.

**In hems:** the box serves its own history locally, `histd` exports everything
on request, and an aggregator's credential is deliberately not allowed to read
either. [Security](@/docs/security.md)

## Glossary

| Term | Meaning |
|---|---|
| **Abgrenzungsoption** | MiSpeL's measured, formula-based separation of green and grey storage flows |
| **BK6-22-300** | The BNetzA determination implementing § 14a EnWG; its Anlage 1 is cited as `[A1 …]` |
| **CEM** | Customer Energy Manager — the S2 role that instructs devices |
| **Failsafe** | The limit a device must apply when the EEBUS link to the Steuerbox goes quiet |
| **Fallgruppe** | One of the four kinds of SteuVE: charge point, heat pump, cooling, storage |
| **Festlegung** | A binding determination of the Bundesnetzagentur |
| **iMSys** | Intelligentes Messsystem — a smart meter with a gateway |
| **LPC / LPP** | EEBUS use cases for limiting consumption and production |
| **MGCP** | EEBUS use case for the grid connection point's measurements |
| **Netzwirksamer Leistungsbezug** | Power drawn from the grid, after the household's own production and storage — what the § 14a limit applies to |
| **Nachweis** | The evidence a network operator may request that a reduction was applied |
| **Pauschaloption** | MiSpeL's flat-rate alternative for roofs up to 30 kWp |
| **SHIP / SPINE** | EEBUS transport and data-model layers |
| **SKI** | Subject Key Identifier — how two EEBUS devices recognise each other |
| **SMGW** | Smart meter gateway, the communication unit of an iMSys |
| **SteuVE** | Steuerbare Verbrauchseinrichtung — a controllable device above 4,2 kW |
| **Steuerbox** | The network operator's control box at the connection point |
