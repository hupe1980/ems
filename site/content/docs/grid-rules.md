+++
title = "German grid rules"
description = "How the gateway applies import and export limits, records reductions and accounts for shared energy."
weight = 6

[extra]
group = "Control and metering"
+++

The household guard applies the limits for the installed site. Inputs include equipment ratings, commissioning dates, metering and control status, and current network operator instructions. The building and industrial demos have their own meter and tariff assumptions.

## § 14a EnWG — which devices, and how far down

A private EV charger, heat pump including auxiliary heaters, space cooling or a battery charging from the grid can count as a controllable consumer (*steuerbare Verbrauchseinrichtung*). For installations covered by the determination, the threshold is more than 4.2 kW. Smaller heat pumps or cooling units of the same category can be grouped for the same operator behind one connection. Two eligible 3 kW heat pumps can count as one 6 kW device. Commissioning date and participation under transitional rules also matter. [Bundesnetzagentur guidance and determination](https://www.bundesnetzagentur.de/DE/Vportal/Energie/SteuerbareVBE/start.html).

For EMS-based control the minimum power depends on the number of eligible devices and the installed heat-pump and cooling capacity. The implementation follows Anlage 1 of BK6-22-300. GZF is the simultaneity factor.

```text
P_min = base + (n − 1) × GZF(n) × 4,2 kW

base = max(0,4 × ΣP_Wärmepumpe ; 0,4 × ΣP_Raumkühlung)  if a heat-pump or
                                                          cooling group above
                                                          11 kW is included
     = 4,2 kW                                             otherwise
```

with the Gleichzeitigkeitsfaktor 0,80 / 0,75 / 0,70 / 0,65 / 0,60 / 0,55 / 0,50
for two to eight devices and 0,45 from nine on.

## The netzwirksamer Leistungsbezug — why a roof lifts the ceiling

The reduction limits the grid draw attributable to eligible devices. A roof producing 4 kW while ordinary loads draw 1.2 kW leaves 2.8 kW of solar for controlled devices. With a 4.2 kW grid-draw ceiling, they can share 7 kW while those readings hold.

A battery discharge can contribute when it is measured, sustained and consistent with the command for this tick. An unavailable controllable device may still be drawing power. The guard accounts for this possible draw and marks its assumption. An unmeasured load cannot create export headroom. Fuses, phase limits and feed-in caps continue to apply. [Guard and shared limits](@/docs/architecture.md#two-kinds-of-limit-and-the-difference-matters).

## The EEBUS limitation machine

The paired Steuerbox sends a limitation to the gateway. The guard uses the accepted instruction and the state of the link.

| State | Meaning |
|---|---|
| `init` | Just restarted; limited by the failsafe value |
| `unlimited/controlled` | In contact, no limit active |
| `limited` | A limit from the Energy Guard is in force |
| `failsafe` | The heartbeat stopped; the failsafe value applies |
| `unlimited/autonomous` | Out of contact long enough that the failsafe was released |

Loss of a required heartbeat changes the applicable limit. After the minimum failsafe duration, continued loss of contact can lead to autonomous operation. Setup shows heartbeat age; the evidence record distinguishes receiving a limit from observing the device’s response. [Device diagnostics](@/docs/devices.md).

## § 9 EEG — two caps with different references

An export limit applies at the connection point. For installations subject to the statutory 60% limit, the reference is installed capacity. An EEBUS factor based on nominal inverter AC power uses a different reference. For a 9.8 kWp roof with an 8 kW inverter, 60% of installed capacity is 5.88 kW. A 50% limit on the inverter rating is 4 kW. The guard applies the binding limit alongside current site restrictions.

The applicable statutory rule depends on equipment, commissioning and remuneration. For plants subject to the transitional cap under § 9(2), installation of metering and control equipment and the network operator’s first successful controllability test are distinct conditions. Record the test outcome when configuring the site. [§ 9 EEG](https://www.gesetze-im-internet.de/eeg_2014/__9.html).

### § 51 — one meter, two rules, two clocks

A cap restricts how much power a site may export. The negative-price rule concerns remuneration for an eligible interval. The gateway keeps metering date, plant eligibility and controllability-test status separate so it can price an interval independently of the physical limit. [Tariff treatment](@/docs/tariffs.md) · [§ 51 EEG](https://www.gesetze-im-internet.de/eeg_2014/__51.html).

## The evidence record

For an individual reduction, inspect its time, requested limit, controlling source and measured response. A device can already be below its permitted ceiling when the instruction arrives. Preserve whether it required a command, how the device answered and which measurements were available. Authorized callers can access event and interval records through histd. [Fleet storage and exports](@/docs/services.md).

## Modul 3 — time-variable network charges

Modul 3 combines Modul 1 with a time-dependent network charge. Standard, low and high windows come from the local network operator’s published price sheet. Configure the operator and effective calendar for the site. A low network-charge window can coincide with an expensive energy price, so compare the combined retail price per interval. [Tariffs](@/docs/tariffs.md#modul-3-time-variable-network-charges) · [Bundesnetzagentur overview](https://www.bundesnetzagentur.de/DE/Vportal/Energie/SteuerbareVBE/start.html).

## MiSpeL — which kilowatt-hour was green

Storage can contain energy from the roof and the grid. MiSpeL defines how eligible installations account for mixed storage when assessing support and levies. The **Abgrenzungsoption** separates quantities using measured flows. The **Pauschaloption** uses a simplified calculation under its own eligibility and activation conditions. The site must declare its method and the dates and consents that apply. [Bundesnetzagentur determination and annexes](https://www.bundesnetzagentur.de/DE/Fachthemen/ElektrizitaetundGas/ErneuerbareEnergien/EEG_Aufsicht/MiSpeL/start.html).

Settlement uses calendar periods and exact decimal quantities. A missing required market or support price leaves the measured energy available, and the settlement route identifies the affected slot. A price of zero is a valid input. Corrected registers retain their earlier versions so an export can be reproduced as of a given instant. [Site history and settlement](@/docs/services.md#site-history-and-settlement).

## § 42c EnWG — sharing electricity with the neighbours

Participants agree an allocation key for sharing eligible electricity over the public distribution network. The generation and consumption in each quarter-hour bound how much a member can receive. In the building demo, a static key leaves some energy unallocated when a member’s demand is below its entitlement. A cascading key can assign that residual to a member with unmet demand. [Community allocation demo](@/docs/simulation.md#compare-community-allocations) · [§ 42c EnWG](https://www.gesetze-im-internet.de/enwg_2005/__42c.html).
