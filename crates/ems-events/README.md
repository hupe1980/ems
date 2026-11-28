# ems-events

Event types, CloudEvents envelopes and Standard Webhooks signatures used by EMS producers and consumers.

Use a catalogue constant for the CloudEvent `type`, for example `SITE_DAY_REPORTED` when a household reports its day to `obsd`. Types are past tense and use the `de.ems` namespace. `cargo xtask check-events` checks new event names against the catalogue.

The webhook signature covers the message ID, timestamp and exact body bytes. Verify those bytes before decoding the payload. The verifier accepts a set of secrets during key rotation and a caller-provided time for replay-window checks.

Parse a configured `whsec_<standard Base64>` secret encoding 24–64 bytes with `webhook::parse_secret` and pass the resulting SDK key to `sign`. Pass the original HTTP headers, body and receiver clock to `verify`. The SDK authenticates the message and enforces its five-minute window. EMS returns the index of the matching site key. The gateway publishes a versioned `dataschema` URI with each subscribed event. [Gateway schemas](https://github.com/hupe1980/ems/tree/main/schemas/events)

[API reference](https://docs.rs/ems-events) · [Guide](https://hupe1980.github.io/ems/docs/services/)

## License

MIT OR Apache-2.0
