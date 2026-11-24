# bemsd

## Replay a charging reduction

`cargo run -p bemsd -- demo` starts two simulated stations and a charging management system on loopback. They exchange OCPP 1.6J messages over WebSockets before the handover opens. [Open the 24 kW request](http://127.0.0.1:8081/?case=delayed-arrival&moment=requested) and use **Weiter** to step through the incident. At 09:12:15 the bank’s delivery is unconfirmed. At handover both station totals are available. West-02 measured at 09:12:15 and its reading arrived at 09:13:20. **Zurück** restores the evidence available at the deadline.

[Replay a station restart](http://127.0.0.1:8081/?case=station-rejoin&moment=deadline). West-02 disconnects while West-01 continues reporting. Step through reconnection and the buffered reading. At 09:13:30 MEZ the first fresh pair totals 26 kW against the original 24 kW request. Rewind to see what was known at the deadline.

**Nachweis je Ladestation** gives each station’s profile response, power and receipt time. Choose **Beispielfall wechseln** for a 26 kW exceedance, a stale reading or an incomplete bank. The [JSON view](http://127.0.0.1:8081/v1/demo/building?case=delayed-arrival&moment=deadline) accepts the same case and moment. Omitting the moment selects handover.

## Heating reserve

Open [Heizreserve](http://127.0.0.1:8081/comfort) in the running demo. At 21.5 °C the bank can draw 24 kW. When the simulated BACnet point flags a fault at the same temperature, the bank retains 6 kW for heating. After 30 seconds without valid evidence, the limit stays at 18 kW. Each row includes both stations’ measured total and the age of the last reliable reading. [JSON receipt](http://127.0.0.1:8081/v1/demo/building/comfort)

In the sensor fault case, open **KNX-Zähler am Ladeabgang vergleichen**. The feeder can corroborate the 18 kW station total, read 4 kW higher, or have an expired observation. The comparison leaves the charging limit at 18 kW.

## Community allocation

Run `cargo run -p bemsd -- community --viewer member-a`. Open [the member view](http://127.0.0.1:8083/) and enter the temporary login printed in the terminal. Choose the second 02:00 interval (+01:00) to see the member’s share. Restart with `--viewer coordinator` to compare both keys. Of the same 10 kWh supply, the static key leaves 3 kWh and the cascading key allocates all 10. Select **Messlücke** to see the final allocation held until the second meter reading arrives. The [JSON view](http://127.0.0.1:8083/v1/demo/community?case=cascading) uses the same credential and allocation.

In the coordinator view, choose **Welche Messung entscheidet?** to reveal either reading in the second 02:00 quarter. A’s 2 kWh proves at least 3 kWh remain under the static key. B alone leaves full allocation possible. Both readings close the allocation.
