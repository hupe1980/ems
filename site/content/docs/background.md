+++
title = "German grid fundamentals"
description = "Who operates the meter, how import and export limits work, and which terms appear on your bill."
weight = 3

[extra]
group = "Start"
+++

A **behind-the-meter energy management system** coordinates devices on the customer’s side of the grid meter. At a house, that can mean moving EV charging to sunny hours. A building operator may instead need to share one connection between chargers and heating. An industrial operator may need to estimate the mean import in a billing interval before it closes.

Power (**kW**) is the current rate of consumption or production. Energy (**kWh**) accumulates over time. A charger drawing 8 kW for 15 minutes uses 2 kWh. A momentary peak of 8 kW, on its own, cannot tell you how much energy went through the meter.

## Who is who

| Role | German term | Responsibility |
|---|---|---|
| Network operator | Verteilnetzbetreiber (VNB) | Operates the distribution network and can send applicable grid restrictions |
| Metering point operator | Messstellenbetreiber (MSB) | Installs and operates the metering equipment |
| Supplier | Lieferant | Sells electricity under the site’s contract |
| Regulator | Bundesnetzagentur (BNetzA) | Issues binding determinations, including the framework for controllable consumption |
| Household or site operator | Anlagenbetreiber or Anschlussnutzer, depending on the role | Provides installation details and authorizes local control |

## § 14a EnWG — the network operator may turn devices down

Eligible controllable consumers on the low-voltage network include private EV chargers, heat pumps, cooling and battery charging. During a reduction, the network operator may limit their grid draw. Ordinary household consumption remains available. In exchange, the household receives a network-charge reduction. The gateway uses the live instruction and measured solar generation to distribute the available power. [Bundesnetzagentur overview](https://www.bundesnetzagentur.de/DE/Vportal/Energie/SteuerbareVBE/start.html) · [How the guard applies the rule](@/docs/grid-rules.md#ss-14a-enwg-which-devices-and-how-far-down).

## The Steuerbox and EEBUS — how the limit arrives

The Steuerbox is the control device at the grid connection. With a supported installation, the gateway receives its instruction over EEBUS. Pairing identifies the peer. Heartbeats tell the gateway whether the control link is still active. Open **Setup** to check the last heartbeat and **Now** to see the effect of an active reduction. [Device connections](@/docs/devices.md).

## Modul 1, 2 and 3 — what the household gets in return

| Module | Network charge arrangement |
|---|---|
| Modul 1 | An annual lump-sum reduction |
| Modul 2 | A reduced energy-based charge for eligible devices measured separately |
| Modul 3 | Modul 1 plus time-variable network charges in the network operator’s published windows |

Compare the modules against how much the devices consume and when they run. The energy component of the supply contract and the network charge can change on different schedules. [Tariffs and prices](@/docs/tariffs.md).

## § 9 and § 51 EEG — the solar peak rules

Section 9 sets technical requirements for feed-in. An eligible installation may have to limit export to a percentage of installed capacity until the metering and control equipment is installed and the network operator has successfully tested controllability. Energy consumed behind the meter does not cross the connection and therefore does not use export headroom.

Section 51 governs support during negative spot prices. A plant’s eligibility and relevant dates determine how an interval is priced. Record the metering installation date and the controllability test separately. [§ 9 EEG](https://www.gesetze-im-internet.de/eeg_2014/__9.html) · [Grid rule examples](@/docs/grid-rules.md#ss-9-eeg-two-caps-with-different-references).

## § 41a EnWG — prices that change every quarter hour

Suppliers must offer a dynamic tariff to customers with an intelligent metering system. The electricity bill can combine a market-linked price with the supplier’s terms, network charges, levies and taxes. A negative wholesale price can still produce a positive retail price. The household gateway uses its effective contract when comparing charging windows. [§ 41a EnWG](https://www.gesetze-im-internet.de/enwg_2005/__41a.html) · [Use a signed contract](@/docs/tariffs.md#use-a-signed-electricity-contract).

## MiSpeL — which kilowatt-hour in the battery was green

A battery can charge from solar and from the grid. MiSpeL defines how to account for these mixed flows under the applicable support and levy rules. The measured approach and the simplified allowance have distinct eligibility, start and consent conditions. A meter reading alone establishes neither eligibility nor the price due. [Determination and annexes](https://www.bundesnetzagentur.de/DE/Fachthemen/ElektrizitaetundGas/ErneuerbareEnergien/EEG_Aufsicht/MiSpeL/start.html) · [Settlement in the gateway and fleet](@/docs/grid-rules.md#mispel-which-kilowatt-hour-was-green).

## § 42c EnWG — sharing with the neighbours

Participants can share eligible renewable electricity over the public distribution network under the applicable agreements, metering requirements and geographic scope. Their agreed allocation key, generation and demand determine each member’s share in a quarter-hour. [§ 42c EnWG](https://www.gesetze-im-internet.de/enwg_2005/__42c.html) · [Try two allocation keys](@/docs/simulation.md#compare-community-allocations).

## S2 and SunSpec — talking to devices and to other managers

S2 describes what flexibility a device can offer. An aggregator can use an authorized S2 session to request a change. A SunSpec device instead exposes measurements and controls on a supported register map. The gateway communicates with these devices over Modbus TCP on the installation network. [S2 manager sessions](@/docs/flexibility.md) · [Household drivers](@/docs/devices.md).

## The EU Data Act — the household’s data

A household’s meter history can reveal occupancy and device activity. The gateway serves its history on the local network, and fleet exports require an authorized credential. Manager tokens for operating devices cannot retrieve the household’s export. [Data access and authorization](@/docs/security.md).

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
| **Netzwirksamer Leistungsbezug** | Grid import attributable to the eligible controllable devices |
| **Nachweis** | The evidence a network operator may request that a reduction was applied |
| **Pauschaloption** | MiSpeL's flat-rate alternative for roofs up to 30 kWp |
| **SHIP / SPINE** | EEBUS transport and data-model layers |
| **SKI** | Subject Key Identifier — how two EEBUS devices recognise each other |
| **SMGW** | Smart meter gateway, the communication unit of an iMSys |
| **SteuVE** | Steuerbare Verbrauchseinrichtung — an eligible controllable consumer; group thresholds can apply |
| **Steuerbox** | The network operator's control box at the connection point |
