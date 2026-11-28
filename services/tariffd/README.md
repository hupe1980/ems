# tariffd

Reconciles public day-ahead prices by source trust, resolution and publication time. The API returns net wholesale prices and cache coverage.

| Endpoint | Result |
|---|---|
| `GET /v1/prices?from=…&slots=96` | the resolved quarter-hourly curve |
| `GET /v1/prices/coverage` | how much of a window it can price |
| `GET /v1/modul3` | which operators the curated Modul 3 catalogue covers |
| `GET /v1/modul3/{netzbetreiber}` | one operator's transcribed calendars |
| `GET /livez`, `/readyz`, `/version` | health and build information |

Configure source credentials as environment or file references. Modul 3 calendars are keyed by operator BDEW code and validated on startup. See [tariffd.example.toml](tariffd.example.toml). [Fleet services guide](https://hupe1980.github.io/ems/docs/services/)

## MCP and the tariff image

The `tariffd` Dockerfile target runs as UID 65532 and reads `/etc/ems/tariffd.toml`. Enable `[mcp]` with `enabled = true` and `token = "env:MCP_TOKEN"` to expose authenticated Streamable HTTP at `/mcp`. The EMS replacement probe checks MCP 2026-07-28 discovery, finds `get_coverage` and asks how many of four quarter-hours the cache can price. [Replacement guide](https://hupe1980.github.io/ems/docs/api/#verify-a-replacement)

## License

MIT OR Apache-2.0
