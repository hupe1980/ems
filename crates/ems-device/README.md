# ems-device

Translate a power request into a command a device can execute within the guard’s envelope. The crate operates on typed assets and commands. Protocol drivers handle the wire format.

| Asset | Translation |
|---|---|
| EV charge point | Amperes per active conductor. Below the configured minimum current the port remains idle. |
| SG Ready heat pump | A supported contact state within the allowed power interval. Check the power associated with each state on the configured unit. |
| PV inverter | A production ceiling. The available sunlight determines actual output. |
| On/off hot-water heater | Run at rated power while the slot needs heat and the guard allows it. |

Use `realisable` for the achievable power and `commands_for` to translate the decision into device commands. For example, a three-phase port with a 6 A minimum requires about 4.14 kW at 230 V. A smaller site allowance must keep it idle.

## Use it

```console
$ cargo add ems-device
```

API reference on [docs.rs](https://docs.rs/ems-device); the design is explained in
[Devices and drivers](https://hupe1980.github.io/ems/docs/devices/).

## License

MIT OR Apache-2.0
