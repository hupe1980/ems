# histd

The fleet historian stores 15-minute meter registers and § 14a control events in PostgreSQL. A network operator can retrieve evidence of the limit it sent and how the site responded. A household can also export its data and settle its declared MiSpeL storage option.

The following site routes start with `/v1/sites/{site}` and require a bearer credential authorized for that site.

| Route | Use |
|---|---|
| `POST /quarter-hours`, `GET /quarter-hours` | Submit a batch of meter registers or read a time window. |
| `POST /events`, `GET /nachweis` | Submit a control event or retrieve § 14a evidence for a time window. |
| `GET /export` | Download the household’s Data Act export. |
| `GET /mispel` | Settle the declared option for a year or, for the Abgrenzungsoption, a year and month. |

Time windows include `from` and exclude `to`. Every HTTP settlement returns its effective RFC 3339 `as_of` cutoff. Save it with the result and pass it back to select the same register versions. An omitted or future cutoff uses the historian’s current time. For a financial comparison, use the same site declaration and rule set. Missing prices preserve measured energy. Settlement returns HTTP 422 and the affected slot if an exported interval needs an unavailable price.

### Submit or correct a register

Send a JSON array to `POST /v1/sites/{site}/quarter-hours`. Each item carries the source revision time and the complete register set. Energy is in kWh. Use decimal strings for quantities and `null` for an unavailable optional reading or price.

```json
[{
  "recorded_at": "2026-11-01T00:15:01.123456Z",
  "registers": {
    "slot": "2026-11-01T00:00:00Z",
    "grid_draw": "0.125",
    "grid_feed_in": "0",
    "device_consumption": "0.125",
    "device_generation": "0"
  }
}]
```

After a lost response, resend the same assertion with its original `recorded_at`. An identical retry returns HTTP 204. Reusing that revision with different values returns HTTP 409 and rolls back the batch. For a correction, issue a later source revision at microsecond precision. Historical settlement uses the historian’s own receipt time, so a late delivery cannot backdate its effect.

Configure PostgreSQL, credentials and the site’s declaration before starting the service. [Configuration example](histd.example.toml) · [Fleet services guide](https://hupe1980.github.io/ems/docs/services/)

### MeterStore registers

To use MeterStore for a site, configure its market binding before its first register submission. Use the site's actual market location and the responsible operator's market-participant identifier.

```toml
[register_meters.haus-1]
malo = "12345678905"
operator = "9900000000003"
```

These identifiers are examples. The existing HTTP and MCP interfaces read the same register history. MeterStore writes the site's assertions to its PostgreSQL hot tier in the historian database. Grid import supports seven decimal places in kWh. Other quantities and prices retain their exact decimal values. The two-year sweep includes both register stores. Sites without a market binding use the historian's ordinary PostgreSQL register table.

MeterStore uses a separate connection pool and PostgreSQL’s `btree_gist` extension. Provision the extension if the historian role cannot create it. Both pools use `[database].tls`. MeterStore verifies the server certificate and hostname when TLS is enabled. For a disposable local database, use `tls = false`. Budget up to `pool_size` connections per pool. Keep the site binding stable across replicas and restarts. Run `just histd-registers` to verify the storage path.

### Operator meter day

The operator desk reads `GET /v1/desk/energy?site=SITE&day=YYYY-MM-DD&days=30` through the OIDC ingress. The window ends on the supplied Berlin date and spans 1–90 calendar days (default 1). Each recorded, closed quarter carries exact decimal kWh for import and export. Use the returned UTC bounds and closure timestamp to distinguish missing quarters from open ones. A corrected register replaces its earlier value. The credential needs both fleet access and read access to that site.
