+++
title = "ems"
template = "index.html"
description = "Open-source energy management for homes, buildings and industry in Germany. Run a HEMS gateway, replay BEMS charging limits or check CEMS peak demand locally."

[extra]

[[extra.faq]]
q = "Which systems can connect to equipment?"
a = "hemsd can run a household gateway with SunSpec/Modbus TCP, EEBUS and S2. It is pre-alpha and has no EEBUS certification. The building and industrial screens use synthetic data. cemsd also accepts local meter traces."

[[extra.faq]]
q = "How does the household gateway use a Steuerbox?"
a = "The Steuerbox sends the network operator’s limit over EEBUS. The household gateway enforces it across controllable devices and records the event."

[[extra.faq]]
q = "Can the household gateway work offline?"
a = "The guard, arbiter and planner run locally. The fleet supplies prices and weather. During a network outage the gateway uses its cached inputs."

[[extra.faq]]
q = "How are household savings calculated?"
a = "Day shows the electricity bill and checks whether the alternative delivered equivalent service. A missed departure or comfort requirement explains an excluded day. Month counts savings only when the difference exceeds the replay error."
+++
