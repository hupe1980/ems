# bemsd

## Replay a charging reduction

`cargo run -p bemsd -- demo` starts two simulated stations and a charging management system on loopback. They exchange OCPP 1.6J messages over WebSockets before the handover opens. [Open the 24 kW request](http://127.0.0.1:8081/?case=delayed-arrival&moment=requested) and use **Weiter** to step through the incident. At 09:12:15 the bank’s delivery is unconfirmed. At handover both station totals are available. West-02 measured at 09:12:15 and its reading arrived at 09:13:20. **Zurück** restores the evidence available at the deadline.

**Nachweis je Ladestation** gives each station’s profile response, power and receipt time. Choose **Beispielfall wechseln** for a 26 kW exceedance, a stale reading or an incomplete bank. The [JSON view](http://127.0.0.1:8081/v1/demo/building?case=delayed-arrival&moment=deadline) accepts the same case and moment. Omitting the moment selects handover.

## Community allocation

Run `cargo run -p bemsd -- community --viewer member-a`. Open [the member view](http://127.0.0.1:8083/) and enter the temporary login printed in the terminal. Choose the second 02:00 interval (+01:00) to see the member’s share. Restart with `--viewer coordinator` to compare both keys. Of the same 10 kWh supply, the static key leaves 3 kWh and the cascading key allocates all 10. Select **Messlücke** to see the final allocation held until the second meter reading arrives. The [JSON view](http://127.0.0.1:8083/v1/demo/community?case=cascading) uses the same credential and allocation.
