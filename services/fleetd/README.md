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

## License

MIT OR Apache-2.0
