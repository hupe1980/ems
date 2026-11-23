# ems-service

Shared HTTP, configuration, authentication and lifecycle support for the EMS services. The building and industrial previews use its loopback demo server.

| Endpoint | What to use it for |
|---|---|
| `/livez` | Check whether the process and its vital background loops are running. |
| `/readyz` | Inspect dependency health and last successful checks before sending traffic. |
| `/version` | Identify the running build. |

Keep liveness and readiness probes separate. An unavailable upstream can make the service unready while its polling loop is healthy. Restarting the service in that state would interrupt recovery.

`config::load_from` accepts an environment lookup supplied by the caller, allowing tests to exercise configuration overrides without changing process-wide variables.

[API reference](https://docs.rs/ems-service) · [Guide](https://hupe1980.github.io/ems/docs/security/)

## License

MIT OR Apache-2.0
