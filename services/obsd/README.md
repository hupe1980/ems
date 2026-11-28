# obsd

Collects signed household day reports and serves fleet summaries. Grid-limit breaches retain the site and date.

| Endpoint | Result |
|---|---|
| `POST /v1/days` | a box reports a day (signed) |
| `GET /v1/fleet` | the summary, with breaches as a list |
| `GET /v1/sites/{site}` | one household's recent days |
| `/mcp` | the same answers for an agent, authorised as its own caller |

Submissions use the Standard Webhooks SDK and its five-minute replay window. Each site needs a distinct `whsec_<standard Base64>` key shared with its gateway. Generate 32 random bytes per key (the accepted range is 24–64 bytes). Add a second key for that site during rotation. Configure site secrets and report retention in [obsd.example.toml](obsd.example.toml). [Fleet services guide](https://hupe1980.github.io/ems/docs/services/)

## Household departure receipt

The [customer portal](../portald/README.md) presents daily energy history and departure receipts. Configure a separate server-side credential per household in `[customer_tokens]`. Both reads resolve the site from that credential.

The gateway sends a signed CloudEvent to `POST /v1/departures`. Customers can read the same evidence as JSON at `GET /v1/me/departure` with their household bearer. The credential identifies the site. An empty receipt returns HTTP 204. The collector keeps the latest departure per site.

## License

MIT OR Apache-2.0
