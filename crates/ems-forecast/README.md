# ems-forecast

Forecasts rooftop PV, household demand and EV charging sessions from weather, site characteristics and local observations. Supply the observation times explicitly to replay a forecast offline.

| Model | Inputs and behaviour |
|---|---|
| PV | Compute solar position and irradiance on the roof plane. Apply cloud cover, inverter clipping and a correction learned from measured production by local hour. |
| Load | Start with the BDEW H25 profile scaled to the household. As observations accumulate, replace the prior with empirical demand by slot and day type. |
| EV | Estimate arrival, departure and required charge by weekday. Sparse session history yields no estimate. |
| Building | Fit a thermal model to indoor temperature, outdoor temperature and heat input. A record with too little variation cannot identify the response. |

Use coverage, bias, MAE and CRPS to score a forecast. PV scoring uses daylight samples. Calibration tracks independent days separately from individual quarter hours so a single well-predicted day cannot establish reliability.

[API reference](https://docs.rs/ems-forecast) · [Guide](https://hupe1980.github.io/ems/docs/forecasting/)

## License

MIT OR Apache-2.0
