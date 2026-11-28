# EMS replacement kit

Run the kit from an extracted release contract bundle or the EMS repository. It requires Python 3.11 or later. The gateway and tariff endpoints may belong to different implementations.

| Provider | Network contract |
|---|---|
| Household gateway | OpenAPI, authenticated household status and a signed structured CloudEvent sent to the probe |
| Tariff service | Authenticated MCP 2026-07-28 discovery, tool listing and a four-quarter-hour coverage query |

Set `HEMS_TOKEN` and `MCP_TOKEN` in the environment. Use a test household with a free subscription slot. Choose a private address on the probe machine that the gateway can reach. Direct HTTPS connections use the system trust store.

```sh
python3 tools/conformance/contracts.py
python3 tools/conformance/probe.py \
  --gateway https://gateway.example --tariff https://tariff.example \
  --callback http://192.168.10.25:8085 --bind 192.168.10.25
```

Exit status zero means all four checks passed. The probe verifies the exact callback bytes and revokes its ping subscription even when verification fails. If the probe process loses power after registration, revoke the subscription named **EMS conformance** at the gateway.

In a full checkout, `just interfaces` builds both OCI archives for the local Docker architecture. To recheck the archives, run `python3 tools/conformance/images.py --archives`. The resulting `target/interfaces/evidence.json` records which role served each port and which immutable image it ran. [Release and deployment guide](https://hupe1980.github.io/ems/docs/api/#verify-a-replacement)
