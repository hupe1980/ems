+++
title = "hems"
template = "index.html"
description = "Open-source home energy management for German households: § 14a, § 9 EEG and MiSpeL as cited code, a grid guard nothing overrides, and honest savings."

[extra]

[[extra.faq]]
q = "Is hems ready to run my house?"
a = "Not yet for an unattended household. It is pre-alpha: the control stack runs real sockets against real devices and is simulated end to end in CI, and it serves its own household page, but it has not been through EEBUS certification. `hemsd demo` shows it on a simulated day."

[[extra.faq]]
q = "Does it replace the network operator's Steuerbox?"
a = "No. The Steuerbox belongs to the metering point operator and sends the § 14a limit. hems is the energy manager behind it: it receives the limit over EEBUS, enforces it across all controllable devices together, and keeps the evidence."

[[extra.faq]]
q = "What hardware does it run on and talk to?"
a = "hemsd runs on a Linux gateway (x86_64 or aarch64). It reads and commands inverters, meters and batteries over SunSpec/Modbus TCP, talks to the Steuerbox and to EEBUS heat pumps, tanks and wallboxes over SHIP, and accepts an external energy manager over S2."

[[extra.faq]]
q = "Does it need the cloud?"
a = "No. Guard, arbiter and planner run on the box. The fleet services only make a plan better with prices and weather; without them the box keeps planning on its cache and the grid limit is enforced locally either way."

[[extra.faq]]
q = "Can my aggregator or another energy manager use it?"
a = "Yes, through S2 (EN 50491-12-2). A manager gets a named credential the household can withdraw, its instructions outrank the box's own plan, and the guard still clamps them to the § 14a limit. It cannot read the household's measured history."

[[extra.faq]]
q = "Who can see my data?"
a = "The household. The box serves its own history locally and the Data Act export comes from histd on request. A network operator can read the § 14a evidence it is entitled to, and nothing else; an aggregator's credential cannot read the history at all."

[[extra.faq]]
q = "How is the saving calculated?"
a = "Against the same household with the same equipment, the same weather and the same grid rules, run the way the hardware runs out of the box. Battery wear, comfort and undelivered service are charged as costs, so the figure is not the flattering bill difference."

[[extra.faq]]
q = "What does it cost?"
a = "Nothing. hems is open source under MIT OR Apache-2.0."
+++
