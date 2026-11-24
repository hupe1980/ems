# ems-drv

Protocol adapters for the EMS edge systems. The caller supplies bytes and timestamps, handles socket I/O and sends outgoing frames. Device drivers report measurements and capabilities. Grid drivers report limits to the guard.

| Feature | Purpose |
|---|---|
| `modbus` | SunSpec and vendor register drivers |
| `eebus` | SHIP/SPINE household and grid-control drivers |
| `ocpp` | Read OCPP 1.6J station inlet power and transaction-scoped outlet energy |
| `matter` | Map signed appliance power for the household gateway |

```mermaid
sequenceDiagram
  participant S as socket (hemsd)
  participant D as Driver (sans-I/O)
  participant R as registry
  S->>D: on_bytes(&[u8], now)
  Note over D: or on_timeout(now) when<br/>poll_deadline passes — which is<br/>where a failsafe is entered
  D-->>R: poll_event() → Measured / GridLimit
  D-->>S: poll_transmit() → bytes to send
  R->>D: command(&Command, now)
```

Read-only SunSpec units reject commands. Writable units rediscover their register map after reconnect and verify each inverter ceiling by readback. An explicitly managed virtual battery accepts bounded model 704 watt targets. Its register confirmation and measured AC power are separate observations.

See the [device guide](https://hupe1980.github.io/ems/docs/devices/) for driver registration, command delivery and measurement freshness.

## License

MIT OR Apache-2.0
