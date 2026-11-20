+++
title = "The box's API"
description = "Every HTTP route hemsd serves, who may call it, and the OpenAPI 3.1 document generated from the handlers' own types."
weight = 16

[extra]
group = "Running it"
+++

`hemsd` serves one local API, on the address in `[api] listen`. The household
page is its first client and uses nothing an integrator cannot. The contract is
an OpenAPI 3.1 document generated from the types the handlers return:

```console
$ hemsd openapi > openapi.json
```

It is also checked in as
[`services/hemsd/openapi.json`](https://github.com/hupe1980/hems/blob/main/services/hemsd/openapi.json),
and the build fails if a response type changes and the document does not.

## Credentials

| Credential | Is | Opens |
|---|---|---|
| **household** | the box's own token, printed at first start and on the handover sheet — as `Authorization: Bearer …`, or as the session cookie a browser gets from `POST /v1/session` | everything |
| **household token** | the same token, but only as a bearer: the cookie is refused | replacing the token, and the sign-in QR code |
| **manager** | a token issued by `PUT /v1/managers/{name}` | status, overrides and S2 — never the household's data |

With the token:

```console
$ curl -k -H "Authorization: Bearer $HEMS_TOKEN" https://box.local:8080/v1/status
```

With the cookie, as a browser does it:

```console
$ curl -k -c jar -H 'Content-Type: application/json' \
    -d "{\"token\":\"$HEMS_TOKEN\"}" https://box.local:8080/v1/session
$ curl -k -b jar https://box.local:8080/v1/home
```

`-k` because the box's certificate is its own; pin its fingerprint from the
Setup view instead where the client allows it. A request on the cookie that
changes anything must carry `Content-Type: application/json`.

## Routes

<!-- api:begin -->
| Method | Path | Credential | What it does |
|---|---|---|---|
| `GET` | `/livez` | none | Whether the process is alive |
| `GET` | `/readyz` | none | Whether the box is ready to control |
| `GET` | `/version` | none | The daemon's name and version |
| `GET` | `/metrics` | none | Prometheus metrics |
| `POST` | `/v1/session` | none | Sign a browser in: exchange the household's token for a cookie |
| `DELETE` | `/v1/session` | none | Sign a browser out |
| `POST` | `/v1/token` | household token | Replace the household's token; the old one and every sign-in stop working |
| `GET` | `/v1/home` | household | The household page's document: now, the plan, goals, the last receipt |
| `GET` | `/v1/status` | household or manager | What the box is doing and what it cannot see |
| `PUT` | `/v1/goals/{asset}` | household | Ask for a charge or a temperature by a time; answered with a verdict |
| `DELETE` | `/v1/goals/{asset}` | household | Withdraw a goal |
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
| `GET` | `/v1/trust` | household | Whether the box kept its identity, and whether its control box is heard |
| `GET` | `/v1/handover` | household | What the handover sheet prints: identities and the § 14a devices |
| `GET` | `/v1/commissioning` | household | The commissioning record, signed by the box: its identity, its control box, its § 14a devices |
| `GET` | `/v1/qr/{what}` | household | A QR code: `ski`, `fingerprint`, `commissioning`, or `signin` (token only, never the cookie) |
| `POST` | `/v1/discover` | household | Look for SHIP nodes, and for SunSpec devices at named private addresses |
| `GET` | `/v1/managers` | household | The energy managers connected to this box |
| `PUT` | `/v1/managers/{name}` | household | Connect a manager; answers its token once |
| `DELETE` | `/v1/managers/{name}` | household | Withdraw a manager's credential |
| `GET` | `/v1/push` | household | The box's push key and the browsers subscribed |
| `POST` | `/v1/push` | household | Subscribe a browser, or change what it is told |
| `DELETE` | `/v1/push` | household | Unsubscribe a browser |
| `POST` | `/v1/push/test` | household | Send one test notification |
| `GET` | `/s2/{asset}` | household or manager | S2 over WebSocket: a manager drives one device, narrowed by the guard |
<!-- api:end -->

An error is a status and, where there is something to say, one sentence of
plain text. `401` means no credential or an unknown one; `403` means a
manager's credential on a route that is the household's.

## A commissioning record a field-service tool can verify

Whoever ordered an installation wants two things back: that the network
operator's control box is paired, and what was installed. The box says both
in one signed statement, which an installer's phone scans from the handover
view or a tool fetches:

```console
$ curl -sk -H "Authorization: Bearer $TOKEN" \
    "https://box.local/v1/commissioning?reference=WO-2026-000417"
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
