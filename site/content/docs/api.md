+++
title = "Edge API reference"
description = "Household HTTP routes, credentials and OpenAPI, plus the read-only building and industrial demo endpoints."
weight = 16

[extra]
group = "Integration and operations"
+++

The household page and third-party integrations use the same local HTTP API. Inspect the gateway’s OpenAPI 3.1 document to see its routes and response types. For HTTPS, compare the certificate’s SHA-256 fingerprint with the one printed at startup before using it as `hems.pem`.

```console
$ curl --cacert hems.pem https://hems.local:8080/openapi.json     # running gateway
$ hemsd openapi > openapi.json                     # or from the binary
```

It is also checked in as
[`services/hemsd/openapi.json`](https://github.com/hupe1980/ems/blob/main/services/hemsd/openapi.json),
and the build fails if a response type changes and the document does not.

## Credentials

| Credential | Is | Opens |
|---|---|---|
| **household** | the box's own token, printed at first start and on the handover sheet — as `Authorization: Bearer …`, or as the session cookie a browser gets from `POST /v1/session` | everything |
| **household token** | the same token, but only as a bearer: the cookie is refused | replacing the token, and the sign-in QR code |
| **manager** | a token issued by `PUT /v1/managers/{name}` | status, overrides and S2 — never the household's data |

Send the household token as a bearer credential.

```console
$ curl --cacert hems.pem -H "Authorization: Bearer $HEMS_TOKEN" https://hems.local:8080/v1/status
```

A browser client can exchange the token for a session cookie.

```console
$ curl --cacert hems.pem -c jar -H 'Content-Type: application/json' \
    -d "{\"token\":\"$HEMS_TOKEN\"}" https://hems.local:8080/v1/session
$ curl --cacert hems.pem -b jar https://hems.local:8080/v1/home
```

A request on the cookie that
changes anything must carry `Content-Type: application/json`.

## Refusals

Every refusal under `/v1/` is an [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)
problem document, `application/problem+json`. Branch on `type`; show `detail`,
which is the sentence the household page shows:

```json
{
  "type": "https://hupe1980.github.io/ems/docs/api/#not-found",
  "title": "There is no such thing on this box",
  "status": 404,
  "detail": "this box does not trust that control box, so there is nothing to forget"
}
```

| `type` | Status | Means |
|---|---|---|
| <span id="bad-request"></span>`#bad-request` | 400 | The request is not one this box can read — malformed JSON, a value that is not a SKI |
| <span id="unauthorized"></span>`#unauthorized` | 401 | No credential, or not one this box issued |
| <span id="forbidden"></span>`#forbidden` | 403 | This credential may not do that — a manager on a household route, a cookie write without a JSON body |
| <span id="not-found"></span>`#not-found` | 404 | No such route, device, goal, subscription or trusted control box; a `DELETE` with nothing to delete |
| <span id="method-not-allowed"></span>`#method-not-allowed` | 405 | That method is not offered on the route |
| <span id="conflict"></span>`#conflict` | 409 | The box is not in a state to do that — a token set in the file, too many subscriptions |
| <span id="too-large"></span>`#too-large` | 413 | The answer would be too large — a completion code beyond what a phone can scan |
| <span id="unsupported-media-type"></span>`#unsupported-media-type` | 415 | The body is not JSON |
| <span id="unprocessable"></span>`#unprocessable` | 422 | Readable, and not something this box can do — the wrong goal for a device |
| <span id="too-many"></span>`#too-many` | 429 | Too many at once |
| <span id="server-error"></span>`#server-error` | 500 | The box failed to do it, usually to keep it |
| <span id="bad-gateway"></span>`#bad-gateway` | 502 | A device or service the box asked did not answer |
| <span id="unavailable"></span>`#unavailable` | 503 | The box cannot answer that yet — no plan, no prices |

A `DELETE` answers `204` when it removed something and `404` when there was
nothing to remove.

## Routes

<!-- api:begin -->
| Method | Path | Credential | What it does |
|---|---|---|---|
| `GET` | `/v1/tariff` | household | Current and next signed supply contract |
| `POST` | `/v1/tariff/preview` | household | Verify the seller file and review its signer, period and price |
| `PUT` | `/v1/tariff` | household | Accept the exact reviewed seller terms |
| `GET` | `/livez` | none | Whether the process is alive |
| `GET` | `/readyz` | none | Whether the box is ready to control |
| `GET` | `/version` | none | The daemon's name and version |
| `GET` | `/openapi.json` | none | This contract: the OpenAPI 3.1 document the box was built with |
| `GET` | `/metrics` | none | Prometheus metrics |
| `POST` | `/v1/session` | none | Sign a browser in: exchange the household's token for a cookie |
| `DELETE` | `/v1/session` | none | Sign a browser out |
| `POST` | `/v1/token` | household token | Replace the household's token; the old one and every sign-in stop working |
| `GET` | `/v1/home` | household | The household page's document: now, the plan, goals, the last receipt |
| `GET` | `/v1/status` | household or manager | What the box is doing and what it cannot see |
| `POST` | `/v1/goals/{asset}/quote` | household | Price a goal or preview charging now against the saved departure |
| `PUT` | `/v1/goals/{asset}` | household | Ask for a charge or a temperature by a time; answered with a verdict |
| `DELETE` | `/v1/goals/{asset}` | household | Withdraw a goal |
| `PUT` | `/v1/goals/{asset}/boost` | household | Confirm immediate charging to the saved car goal |
| `DELETE` | `/v1/goals/{asset}/boost` | household | Return the car to its plan |
| `GET` | `/v1/overrides` | household or manager | The overrides in force |
| `DELETE` | `/v1/overrides` | household or manager | Withdraw every override |
| `PUT` | `/v1/overrides/{asset}` | household or manager | Boost, pause or mark away; the guard still narrows it |
| `DELETE` | `/v1/overrides/{asset}` | household or manager | Withdraw one override |
| `GET` | `/v1/series/{point}` | household | The box's own measurements of `grid` or a device |
| `GET` | `/v1/days` | household | The closed days the box keeps, at most ninety, and their sum |
| `GET` | `/v1/days.csv` | household | The same days as CSV |
| `GET` | `/v1/pairing` | household | The control boxes this box trusts, and those asking |
| `POST` | `/v1/pairing` | household | Trust a control box by its SKI |
| `DELETE` | `/v1/pairing/{ski}` | household | Stop trusting a control box |
| `POST` | `/v1/pairing/{ski}/refuse` | household | Refuse a control box that is asking |
| `GET` | `/v1/trust` | household | Installer diagnostics for identity, devices and quarter-hour history |
| `GET` | `/v1/handover` | household | What the handover sheet prints: identities and the § 14a devices |
| `GET` | `/v1/commissioning` | household | The commissioning record, signed by the box: its identity, its control box, its § 14a devices |
| `GET` | `/v1/qr/{what}` | household | A QR code: `ski`, `fingerprint`, `commissioning`, or `signin` (token only, never the cookie) |
| `POST` | `/v1/commissioning/first-meter` | household | Preview a connection meter and optional observed assets on an empty site |
| `GET` | `/v1/commissioning/transfer` | household | Read the controller handover review |
| `PUT` | `/v1/commissioning/transfer` | household | Authorize or revoke an installer handover |
| `GET` | `/v1/commissioning/change` | household | Read the latest installation outcome |
| `POST` | `/v1/commissioning/change` | household | Activate the reviewed driver addition |
| `POST` | `/v1/discover` | household | Look for SHIP nodes, and for SunSpec devices at named private addresses |
| `GET` | `/v1/managers` | household | The energy managers connected to this box |
| `PUT` | `/v1/managers/{name}` | household | Connect a manager; answers its token once |
| `DELETE` | `/v1/managers/{name}` | household | Withdraw a manager's credential |
| `GET` | `/v1/push` | household | The box's push key and the browsers subscribed |
| `POST` | `/v1/push` | household | Subscribe a browser, or change what it is told |
| `DELETE` | `/v1/push` | household | Unsubscribe a browser |
| `POST` | `/v1/push/test` | household | Send one test notification |
| `GET` | `/v1/subscriptions` | household | The systems this box tells about its events |
| `POST` | `/v1/subscriptions` | household | Tell one more system: CloudEvents signed per Standard Webhooks |
| `POST` | `/v1/subscriptions/{id}/revoke` | household | Stop telling one, and forget what it was owed |
| `POST` | `/v1/subscriptions/{id}/test` | household | Send it a de.ems.test.ping now |
| `GET` | `/v1/deliveries` | household | The last seven days of deliveries, newest first |
| `GET` | `/s2/{asset}` | household or manager | S2 over WebSocket: a manager drives one device, narrowed by the guard |
<!-- api:end -->

An error is a status and, where there is something to say, one sentence of
plain text. `401` means no credential or an unknown one; `403` means a
manager's credential on a route that is the household's.

## Household readings

For `GET /v1/home`, a saved car or tank goal is returned with its feasibility
assessment. The second tuple element is null while the necessary device
telemetry is missing. The first element still contains the saved settings.

On an empty gateway, `POST /v1/commissioning/first-meter` validates the
three-phase meter, fixed tariff and support contact. Optional `roof` and
`storage` entries require separate verified units and an
`external_limit_controller` for shadow mode. The response contains the exact
initial TOML and a confirmation id for the installation endpoint.

For `POST /v1/discover`, supply `hosts`, optional `unit` (default 1) and
`port` (502 or 1502). A newly found SunSpec inverter with one of models 101–103
and 123 can include `review`. It contains the proposed driver TOML, address,
unit and a nullable configuration refusal. Where Apply is available, send only
the issued `proposal` as `{"id":"…"}` to `POST /v1/commissioning/change`.
A changed file or unknown id returns 409. Read the latest receipt with
`GET /v1/commissioning/change` after reconnection. Retrying the same id
returns its recorded outcome.

When the household Matter pilot is configured, `GET /v1/home` includes `matter` with its node, endpoint, signed watts, receipt time and status. The Setup card labels a failed read as stale when an earlier observation exists.

The home receipt and closed-day history use whole Wh and cents. `metered_bill`
reports priced, expected and observed-mode intervals. Its `energy_ct` and
`fixed_ct` fields are null when a bill cannot be computed. `money` contains
simulation economics. On measured days, nullable `comparison` contains
`with_plan_ct`, `without_plan_ct` and the summed absolute interval error
`error_ct`. Month totals exclude differences at or below that error. Nullable
`arbitrage` records grid charging and household discharge for closed cycles. `forecast` reports
the measured percentage of production and load readings inside their forecast
ranges. `shadow: true` on the home document identifies an observing gateway.

Households can read `GET /v1/commissioning/transfer` while observing and
submit its current `id` and boolean `approve` to `PUT` on the same route.
Authorization lasts one hour and can be revoked until the installer switches
the configured controller and restarts.

## Events out

The box tells the systems a household names what it tells the household —
a § 14a reduction, failsafe, a goal at risk, a device not following, the
closed day — with the numbers. Each is a
[CloudEvent 1.0](https://cloudevents.io) in structured JSON, `POST`ed with
`Content-Type: application/cloudevents+json` and signed per
[Standard Webhooks](https://www.standardwebhooks.com): `webhook-id` (the
event's `id`, the same on every retry), `webhook-timestamp` and
`webhook-signature`, keyed with the `whsec_` secret the box shows once when
the subscription is made. Every webhook library verifies it as it is.

```console
$ curl --cacert hems.pem -H "Authorization: Bearer $HEMS_TOKEN" -H 'Content-Type: application/json' \
    -d '{"name":"Home Assistant","url":"http://homeassistant.local:8123/api/webhook/replace-with-your-random-webhook-id","types":["de.ems.grid.*"]}' \
    https://hems.local:8080/v1/subscriptions
```

| Type | When | `data` |
|---|---|---|
| `de.ems.grid.lpc.limit.received` | the network operator reduces the connection | `limit_w`, `minimum_w` |
| `de.ems.grid.lpc.limit.released` | the reduction ends | `lasted_s` |
| `de.ems.grid.lpc.failsafe.entered` | the control box falls silent | `failsafe_w` |
| `de.ems.grid.lpc.failsafe.left` | the failsafe ends | `reduced` |
| `de.ems.grid.lpp.limit.received` | the export ceiling changes | `limit_w` |
| `de.ems.device.lost` | a device stops reporting | `last_seen` |
| `de.ems.site.goal.at-risk` | a goal turns *tight* or *impossible* | `verdict`, `goal`, `reachable` |
| `de.ems.device.deviation.detected` | a device ignores the box for ten minutes | `commanded_w`, `measured_w` |
| `de.ems.site.day.closed` | a day closes | the receipt: whole cents, Wh, the § 14a lines |
| `de.ems.test.ping` | `POST /v1/subscriptions/{id}/test` | `test` |

`types` takes a trailing `*` (`de.ems.grid.*`); `subjects` narrows to asset
ids. The schema of every `data` is in `openapi.json`'s `webhooks` section.

For an episode that also triggers a browser notification, `notification.data.event_id` matches the CloudEvent `id`, including after quiet hours.

A receiver answers `2xx`. Anything else is tried again — after 30 s, 2 min,
10 min, 30 min, 1 h and then every 3 h — for 24 hours, then dropped;
`GET /v1/deliveries` shows the last seven days of attempts. The box delivers
to an address on the household's network over `http` or `https`, to one on
the internet over `https` only and only with `"public": true`, and never to
loopback, link-local or itself; it follows no redirect. At most eight
systems.

### Home Assistant

Use the same random webhook ID in the subscription URL and the automation. Keep `local_only` enabled. [Home Assistant webhook triggers](https://www.home-assistant.io/docs/automation/trigger/#webhook-trigger).

{% raw %}
```yaml
automation:
  - alias: "hems: the network operator reduces the connection"
    triggers:
      - trigger: webhook
        webhook_id: replace-with-your-random-webhook-id        # long and random: it is the secret here
        allowed_methods: [POST]
        local_only: true
    conditions:
      - condition: template
        value_template: "{{ trigger.json.type == 'de.ems.grid.lpc.limit.received' }}"
    actions:
      - action: persistent_notification.create
        data:
          title: "§ 14a"
          message: >-
            Reduced to {{ (trigger.json.data.limit_w / 1000) | round(1) }} kW.
            {% if trigger.json.data.minimum_w is number %}
            {{ (trigger.json.data.minimum_w / 1000) | round(1) }} kW stay guaranteed.
            {% endif %}
```
{% endraw %}

## A commissioning record a field-service tool can verify

Whoever ordered an installation wants two things back: that the network
operator's control box is paired, and what was installed. The box says both
in one signed statement, which an installer's phone scans from the handover
view or a tool fetches:

```console
$ curl -sS --cacert hems.pem -H "Authorization: Bearer $TOKEN" \
    "https://hems.local:8080/v1/commissioning?reference=WO-2026-000417"
eyJhbGciOiJFUzI1NiIsInR5cCI6ImhlbXMtY29tbWlzc2lvbmluZytqd3MiLC…
```

It is a JWS in compact serialisation (RFC 7515), signed ES256 with the key
behind the box's SKI. `reference` is the caller's own identifier for the
work — 1 to 64 printable ASCII characters without a space — and comes back
byte for byte; the box never reads it.

| Protected header | |
|---|---|
| `alg` | `ES256` |
| `typ` | `hems-commissioning+jws` |
| `x5t#S256` | SHA-256 of the box's SHIP certificate, base64url |
| `x5c` | that certificate, DER in base64 — on `/v1/commissioning`, not in the QR code |

| Payload | |
|---|---|
| `v` | `1` |
| `reference` | as given; absent if none was |
| `issued_at` | when the box signed, RFC 3339 UTC |
| `box` | `ski` (40 lower-case hexadecimal characters) and `ship_id` |
| `control_box` | `state` — `none`, `unpaired`, `silent`, `failsafe` or `heard` — and its `ski` where exactly one is trusted |
| `identity` | `kept` if a restart restores this SKI, `ephemeral` if not |
| `module` | the § 14a module, as configured |
| `minimum_w` | the power a reduction may not go below, watts |
| `devices` | each controllable device: `fallgruppe`, `letter`, `power_w`, `minimum_direct_w`, `commissioned` where stated, and — not in the QR code — the box's own names for its `assets` |

Fetching is the primary path and has no size limit: the record grows by
under 200 octets with each device. `GET /v1/qr/commissioning?reference=…` is
the shortcut — the same record as a QR code, without `x5c` and `assets` so
that a phone can read it: about 1.3 kB for four devices, and at most 1 732
octets (QR version 30), which a house with six or seven devices stays under.
A larger installation answers `413` and is fetched instead.

To verify one:

1. Check the ES256 signature over `header.payload` with the public key of
   the certificate in `x5c` — or, for a scanned code, of the certificate
   whose SHA-256 is `x5t#S256`.
2. Compare that certificate's SKI with `box.ski` and with the SKI on the
   order. A record signed by another box is a valid signature and the wrong
   house.
3. Check `typ` and that `issued_at` is recent enough for the purpose.
4. Only then compare `reference` with the order. A record that verifies but
   names another order — or none: the field is absent when the box was
   commissioned before the order existed, or nobody entered it — is a
   different failure from an invalid signature, and should be reported as
   one: the box is genuine and the record has to be issued again.
5. Read it for what it is: the signature proves this box said so at that
   time. It does not prove the work was done well.

## Building demo

`bemsd demo` serves `GET /v1/demo/building?case=unconfirmed` and the
corresponding page at `/?case=unconfirmed`. Supported cases are
`confirmed` (default), `delayed-arrival`, `above-request`, `unconfirmed`, `stale-charger`, `stale`, `over-limit`, `unknown-pv` and `no-solar`.
Choose `moment=requested`, `observed`, `deadline` or `handover` on either route. The default is `handover`. The connection snapshot first appears at handover. Until the station acknowledges, `profile_accepted` is null. Each station’s `final_received_at` distinguishes receipt from observation. In `case=delayed-arrival` a late sample confirms the deadline reading at handover.

For the charging incident, `charging_source` is `ocpp-1.6j-station-telemetry`. Each member of `stations` gives the station ID, profile acknowledgement, initial kW and final kW. A null contribution makes the corresponding bank reading null. Station totals exclude individual connectors.

The `delivery` field compares `final_sample_kw` to `requested_kw`. Its values are `pending` before the deadline, `within-limit`, `above-limit` and `unconfirmed`.
For a missing final reading, `final_sample_at` and `final_sample_kw` are null. `final_sample_due_at` preserves the expected time.
A missing current measurement or an unconfirmed final sample is JSON null.
The live gateway does not mount either route.

## Community allocation demo

`bemsd community` serves `GET /v1/demo/community` on loopback port 8083. Use the temporary Basic login printed at startup. Select `case=static` or `case=cascading` to compare keys. Select `interval=1` or `2` for the earlier or later occurrence of 02:00. Members receive only their own rows. The coordinator also receives coverage and residual energy in kWh.

## Industrial demo

`cemsd demo` serves `GET /v1/demo/industrial?moment=early`. Use
`early`, `delayed` or `closed` for the same shift. At close, projection
and intervention fields are null and `closed_mean_kw` contains the simulated
meter outcome. Add `reveal=true` to an early or delayed decision to compare its projection with the original synthetic close. The HTML view takes the same query at `/`.

Use `moment=unavoidable` to inspect an interval whose minimum already exceeds
the prior peak. Optional `evidence` values are `complete`, `stale-meter`,
`battery-unknown`, `tariff-review`, `limited-battery`, and `missing-close`. Each suppresses
only the estimates that depend on the missing input or applies the fixture's
60 kW limit. Reloading the same URL reproduces the same evidence.


## Local power history

With a household credential, request one complete quarter at the grid connection:

```http
GET /v1/series/grid?from=2026-11-23T10:00:00Z&to=2026-11-23T10:15:00Z&resolution=quarter_hours
```

`at` identifies the quarter start. `watts` is the arithmetic mean of the observations. `min_watts`, `max_watts` and `samples` describe their range and count. Positive watts indicate import at the grid connection. A quarter with no observations has no row. A measured zero has a row.

For explicit summaries, supplied boundaries must fall on a quarter hour. With just `?resolution=quarter_hours`, the response covers the four most recently closed quarters. Omit `resolution` to select the available tier automatically. Windows include their start, exclude their end and may span up to 1,096 days. A box with local history disabled returns 404. During another history read it returns 503. Retry after the first request finishes.
