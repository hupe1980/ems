+++
title = "Gateway and fleet security"
description = "Verify device identity, configure TLS and access tokens, and check release provenance."
weight = 15

[extra]
group = "Integration and operations"
+++

A household installation has three identities to check: the gateway serving the browser, the grid operator’s control box, and any energy manager allowed to request device schedules. Each has its own credential and revocation path.

## Gateway trust and pairing

Use the household credential to sign in to **Setup**. Compare the gateway’s HTTPS certificate fingerprint with the installer’s handover record. For SHIP pairing, compare the waiting control box’s subject key identifier (SKI) with the one provided by the metering operator before approving it. An unknown peer waits for approval before exchanging control data.

The gateway persists its SHIP key and trusted peers across restarts. Protect its storage and backups. Someone who obtains the private key can impersonate the gateway. Keep Modbus equipment on a restricted network because standard Modbus TCP reads and writes carry no caller identity. [Device guide](@/docs/devices.md).

External energy managers receive individually revocable bearer credentials. Their S2 instructions are attributed to the authenticated subject. Household-only actions include approving peers, editing goals and adding webhook subscribers. A signed-in wall display holds a household session, so place it where only authorized people can use it. Treat the printed sign-in code as a credential. [Household access and handover](@/docs/household.md).

## Fleet access

Fleet services check the caller’s capabilities, tenant and site. A site credential reads and writes its own records. An operator can inspect evidence in its assigned tenant. The household’s detailed export is a separate permission.

| Capability | Resource |
|---|---|
| `hems.record.read` | Site-scoped compliance evidence |
| `hems.record.write` | Submission of site records |
| `hems.export.read` | The household’s Data Act export |
| `hems.fleet.read` | Fleet summaries, roster or advice within scope |

Operational probes and metrics are available to the deployment network. Restrict access at the network boundary. HTTP metrics use matched route patterns, so they can be aggregated without putting household IDs in labels. The [fleet service guide](@/docs/services.md) covers the role of each daemon.

## Secrets and transport

Configure deployed credentials through environment or file references. A missing reference causes startup to fail. This example points `obsd` to a key mounted by its deployment.

```toml
[webhook_secrets]
haus-1 = ["file:/run/secrets/haus-1"]
```

Use platform TLS roots for public data providers. A private fleet can restrict outbound trust to its own CA with a pinned bundle. Pinning replaces the platform root store for that client.

```toml
[service.http.tls_roots]
from = "pinned"
bundle = "/etc/hems/fleet-ca.pem"
```

Fleet PostgreSQL connections use TLS by default. Encryption at rest, backup protection and access to restores are deployment responsibilities. Standard Webhooks signatures bind fleet day reports to the sending site, the payload and its delivery timestamp. A signature establishes provenance; meter accuracy depends on the measurement chain.

## Release verification

The release workflow packages `hemsd`, `bemsd` and `cemsd` separately for x86-64 and ARM64 Linux. Each daemon has a CycloneDX inventory. Archive checksums and provenance attestations accompany the downloads. From the directory containing a downloaded gateway archive, verify its origin against the project:

```sh
gh attestation verify hemsd-*.tar.gz --repo hupe1980/ems
```

Use `cargo audit bin hemsd` on an extracted binary to inspect its embedded Rust dependency data against advisories. Install gateway software and configuration through the installer or operating system. Fleet services can distribute signed manifests; the gateway has no automatic installer for those manifests.

## Decommissioning

Forward queued evidence before a factory reset. The gateway refuses ordinary reset while evidence is still owed. Reset removes local credentials, trusted peers, household history and subscriptions. Records already forwarded remain subject to the fleet’s separate retention policy. [Security controls and residual risks](https://github.com/hupe1980/ems/blob/main/THREAT_MODEL.md).
