# agentd

Read-only fleet advice. Each review uses one snapshot from `obsd` and journals its input. Specialists can read fleet summaries and records. They have no device-control or household export permission.

| Specialist | Findings |
|---|---|
| `compliance-triage` | Groups § 14a breaches by date and recurring feed-in breaches by site. |
| `saving-provenance` | Reports which days support a savings estimate and whether forecast calibration has enough independent days. |

| Interface | Result |
|---|---|
| `GET /v1/advice` | the queue: what each specialist last concluded, when, over how many days, and the run that produced it |
| `/mcp` | the same queue for an agent — `list_advice`, `list_specialists` |

Reading the queue requires `hems.fleet.read`. [Configuration example](agentd.example.toml)

## Try it

```console
$ just agent-demo
```
