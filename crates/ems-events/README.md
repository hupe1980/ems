# ems-events

Event types, CloudEvents envelopes and Standard Webhooks signatures used by EMS producers and consumers.

Use a catalogue constant for the CloudEvent `type`, for example `SITE_DAY_REPORTED` when a household reports its day to `obsd`. Types are past tense and use the `de.ems` namespace. `cargo xtask check-events` checks new event names against the catalogue.

The webhook signature covers the message ID, timestamp and exact body bytes. Verify those bytes before decoding the payload. The verifier accepts a set of secrets during key rotation and a caller-provided time for replay-window checks.

[API reference](https://docs.rs/ems-events) · [Guide](https://hupe1980.github.io/ems/docs/services/)

## License

MIT OR Apache-2.0
