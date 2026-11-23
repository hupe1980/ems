+++
title = "Agents and MCP"
description = "Connect read-only MCP clients to fleet services and review advisory proposals with scoped credentials."
weight = 14

[extra]
group = "Integration and operations"
+++

Fleet operators can read energy data through the Model Context Protocol (MCP), or run a scheduled review with `agentd`. MCP exposes read tools on fleet daemons. The review specialists are deterministic Rust functions and run without a language model.

## Try a review

From the repository root, run the local review example:

```sh
just agent-demo
```

Look for the observation window, affected sites and journal run attached to each finding. A cluster of breaches on the same day is a reason to inspect that day’s commands. It does not establish which device caused a breach.

## Review specialists

| Specialist | What to inspect |
|---|---|
| `compliance-triage` | Below-minimum commands grouped by date, recurring feed-in breaches grouped by site, and overlap between breaches and periods without a plan |
| `saving-provenance` | Which days contributed to a savings estimate, which were excluded, and whether the forecast sample is large enough for calibration |

Each review reads one fleet summary from `obsd` using a tenant-scoped operator credential. Both specialists receive the same snapshot. The journal records the input and run identifier so the result can be replayed. Back up the journal before changing its runtime version.

## Read the queue

Use `GET /v1/advice` with an operator credential carrying `hems.fleet.read`. It returns the latest findings and reports which specialists have run. The optional `/mcp` endpoint exposes `list_advice` and `list_specialists`. A specialist with no completed review has no result to report.

An operator can use the findings to investigate through fleet tools. The advice service has no path to device controls. Its specialists cannot acquire household export permission or write evidence. MCP calls use the caller’s own authority. [Fleet services](@/docs/services.md) · [Access control](@/docs/security.md).

## Configure the daemon

Set `obsd`, `obsd_token`, `tenant` and a persistent `journal` path in the service configuration. For multi-tenant deployments, use an explicit tenant and site membership. The example runs reviews every six hours and keeps MCP disabled until configured. [Example configuration](https://github.com/hupe1980/ems/blob/main/services/agentd/agentd.example.toml).
