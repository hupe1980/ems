# forecastd

Serves quarter-hourly irradiance and temperature by location. The household gateway uses its own PV model and roof correction.

| Endpoint | Result |
|---|---|
| `GET /v1/weather/{location}` | irradiance and temperature, quarter-hourly |
| `GET /v1/production/{location}?kwp=9.8` | the *geometric* model applied to a nameplate — a starting point, not a forecast |
| `GET /livez`, `/readyz`, `/version` | health and build information |

Readiness measures coverage of the requested horizon. Configure upstream locations and polling in [forecastd.example.toml](forecastd.example.toml). [Fleet services guide](https://hupe1980.github.io/ems/docs/services/)

## License

MIT OR Apache-2.0
