# obsd

Collects signed household day reports and serves fleet summaries. Grid-limit breaches retain the site and date.

| Endpoint | Result |
|---|---|
| `POST /v1/days` | a box reports a day (signed) |
| `GET /v1/fleet` | the summary, with breaches as a list |
| `GET /v1/sites/{site}` | one household's recent days |
| `/mcp` | the same answers for an agent, authorised as its own caller |

Submissions use Standard Webhooks signatures. Configure site secrets and report retention in [obsd.example.toml](obsd.example.toml). [Fleet services guide](https://hupe1980.github.io/ems/docs/services/)

## License

MIT OR Apache-2.0
