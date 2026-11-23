# ems-flex

Translate household assets into S2 resource descriptions and a manager’s instructions back into device commands. S2 lets a Customer Energy Manager schedule a resource using its operating limits, stored energy or available programmes. The wire types come from [s2-kit](https://github.com/hupe1980/s2-kit).

| Control type | Resource represented |
|---|---|
| Fill Rate Based Control (FRBC) | A battery, hot-water tank or EV with a known storage session. Charge and discharge rates account for efficiency. |
| Power Envelope Based Control (PEBC) | A bounded charge point, curtailable PV inverter or heat pump with a power ceiling. |
| Operation Mode Based Control (OMBC) | A device with discrete operating states, such as an SG Ready heat pump. |
| Power Profile Based Control (PPBC) | An appliance with a fixed programme and a permitted start window. |

Use `describe_site` to enumerate the supported descriptions and the assets it could not describe. Resource and operating-mode IDs derive from the asset identity so a reconnect preserves the references a manager has cached.

`Session` handles negotiation, scheduled instructions, revocation and status reporting. The gateway owns the transport and checks whether an accepted instruction can reach an actuator. [S2 standard](https://s2standard.org)

[API reference](https://docs.rs/ems-flex) · [Guide](https://hupe1980.github.io/ems/docs/flexibility/)

## License

MIT OR Apache-2.0
