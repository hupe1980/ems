# fleetd

Fleet enrolment, configuration and release manifests. An installer provisions a single-use enrolment secret. A client exchanges it for a site credential and reports the configuration version it runs.

The service distributes Ed25519 signatures created by an external signer. Install household configuration and updates through the installer or operating system. [Configuration example](fleetd.example.toml)

| Route | Result |
|---|---|
| `POST /v1/enrol` | single-use enrolment |
| `GET /v1/config` | the signed configuration for this box |
| `POST /v1/config/running` | which version it is actually running |
| `GET /v1/releases/{component}` | a signed release manifest |
| `GET /v1/fleet` | what the fleet is enrolled on |

## Operator desk

Open `/desk` through the [OIDC ingress](../../deploy/oidc/README.md). Filter to **Needs attention** or search by site. The desk shows 15 sites per page, prioritized by condition. Expand a site to load up to 32 recent receipts in UTC.

<img src="../../site/static/screens/operator-desk.png" width="800" alt="A fleet of 100 sites filtered to the one with a stale feeder reading">

Gateways post to `/v1/checkins` with their site bearer. Send `sent_at` every 30 seconds, with optional `feeder_at` and `report_at` in Unix seconds. Synchronize the gateway clock. The fleet accepts sends up to 90 seconds old and ignores duplicates or out-of-order sends. Omit an observation when its time is unknown. A closure is reported by the gateway.

To rehearse a stalled feeder, set `FLEET_SITE_TOKEN` to an enrolled simulator's credential and run:

```sh
python3 xtask/fleet_checkin.py http://127.0.0.1:7180 --stalled-feeder --count 3
```

Refresh the desk during the rehearsal. Gateway contact stays current while the feeder reading ages. After the simulator stops, the gateway becomes silent once receipt age exceeds 90 seconds. Feeder readings age out after 120 seconds and report closures after 36 hours. `GET /v1/desk` returns scoped summaries. Add `?site=SITE` for one site’s receipt history.

## License

MIT OR Apache-2.0
