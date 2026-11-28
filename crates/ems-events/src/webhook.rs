//! Authentication of structured EMS events with the Standard Webhooks SDK.

use http::HeaderMap;
use time::OffsetDateTime;

pub use standardwebhooks::{
    HEADER_WEBHOOK_ID as ID_HEADER, HEADER_WEBHOOK_SIGNATURE as SIGNATURE_HEADER,
    HEADER_WEBHOOK_TIMESTAMP as TIMESTAMP_HEADER, Webhook, WebhookError,
};

/// A configured fleet signing secret could not be decoded.
#[derive(Debug, thiserror::Error)]
#[error("expected whsec_ followed by a standard Base64 key of 24 to 64 decoded bytes")]
pub struct InvalidSigningSecret;

/// Parse the secret shared by a gateway and its fleet receiver.
///
/// # Errors
/// Returns a redacted error when the prefix or Base64 key is invalid.
pub fn parse_secret(value: &str) -> Result<Webhook, InvalidSigningSecret> {
    let encoded = value.strip_prefix("whsec_").ok_or(InvalidSigningSecret)?;
    // The Standard Webhooks symmetric profile specifies 24–64 key bytes.
    // Each Base64 symbol carries six bits. The SDK checks the alphabet and padding.
    let symbols = encoded.trim_end_matches('=').len();
    if !(32..=86).contains(&symbols) {
        return Err(InvalidSigningSecret);
    }
    Webhook::new(value).map_err(|_| InvalidSigningSecret)
}

/// The headers a signed request carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    /// The message id, which is the CloudEvent's `id`.
    pub id: String,
    /// The signing timestamp, Unix seconds.
    pub timestamp: i64,
    /// `v1,<base64>`.
    pub signature: String,
}

impl Signed {
    /// The three headers, ready to be written onto a request.
    #[must_use]
    pub fn headers(&self) -> [(&'static str, String); 3] {
        [
            (ID_HEADER, self.id.clone()),
            (TIMESTAMP_HEADER, self.timestamp.to_string()),
            (SIGNATURE_HEADER, self.signature.clone()),
        ]
    }
}

/// Sign the exact event bytes using the caller's clock.
///
/// # Errors
/// Returns an SDK error if the payload is invalid.
pub fn sign(
    key: &Webhook,
    id: &str,
    at: OffsetDateTime,
    body: &[u8],
) -> Result<Signed, WebhookError> {
    let timestamp = at.unix_timestamp();
    Ok(Signed {
        id: id.to_owned(),
        timestamp,
        signature: key.sign(id, timestamp, body)?,
    })
}

/// Verify the request and identify the configured key that signed it.
///
/// # Errors
/// Returns an error when the SDK rejects the request for every key, or when
/// the timestamp is outside the range of a representable EMS instant.
pub fn verify<'a>(
    keys: impl IntoIterator<Item = &'a Webhook>,
    headers: &HeaderMap,
    body: &[u8],
    now: OffsetDateTime,
) -> Result<usize, WebhookError> {
    // SDK 1.3 subtracts the wire timestamp from now using signed arithmetic.
    // Bound both operands to the time crate's representable range first.
    let timestamp = headers
        .get(TIMESTAMP_HEADER)
        .ok_or(WebhookError::MissingHeader("timestamp"))?;
    let seconds = timestamp
        .to_str()
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or(WebhookError::InvalidTimestamp)?;
    OffsetDateTime::from_unix_timestamp(seconds).map_err(|_| WebhookError::InvalidTimestamp)?;

    let mut result = Err(WebhookError::InvalidSignature);
    for (index, key) in keys.into_iter().enumerate() {
        match key.verify_with_timestamp(body, headers, now.unix_timestamp()) {
            Ok(()) => result = Ok(index),
            Err(error) if result.is_err() => result = Err(error),
            Err(_) => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Duration, macros::datetime};

    const SECRET: &str = "whsec_C2FVsBQIhrscChlQIMV+b5sSYspob7oD";
    const NOW: OffsetDateTime = datetime!(2022-04-07 23:25:53 UTC);
    const BODY: &[u8] = br#"{"email":"test@example.com","username":"test_user"}"#;

    fn headers(signed: &Signed) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in signed.headers() {
            headers.insert(name, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn sdk_vector_and_time_extremes() {
        let key = parse_secret(SECRET).unwrap();
        let at = OffsetDateTime::from_unix_timestamp(1_649_367_553).unwrap();
        let signed = sign(&key, "msg_27UH4WbU6Z5A5EzD8u03UvzRbpk", at, BODY).unwrap();
        assert_eq!(
            signed.signature,
            "v1,tZ1I4/hDygAJgO5TYxiSd6Sd0kDW6hPenDe+bTa3Kkw="
        );
        let mut headers = headers(&signed);
        assert_eq!(verify([&key], &headers, BODY, at), Ok(0));
        for seconds in [-301, -300, 300, 301] {
            assert_eq!(
                verify([&key], &headers, BODY, at + Duration::seconds(seconds)).is_ok(),
                seconds.abs() <= 300
            );
        }
        for timestamp in [i64::MIN, i64::MAX] {
            headers.insert(TIMESTAMP_HEADER, timestamp.to_string().parse().unwrap());
            assert_eq!(
                verify([&key], &headers, BODY, at),
                Err(WebhookError::InvalidTimestamp)
            );
        }
    }

    #[test]
    fn rotation_checks_the_actual_sender_and_all_offered_signatures() {
        let retiring = parse_secret(SECRET).unwrap();
        let current = Webhook::from_bytes(vec![42; 32]).unwrap();
        for (index, key) in [&retiring, &current].into_iter().enumerate() {
            let signed = sign(key, "msg", NOW, BODY).unwrap();
            let mut headers = headers(&signed);
            headers.insert(
                SIGNATURE_HEADER,
                format!("v2,other v1,invalid {}", signed.signature)
                    .parse()
                    .unwrap(),
            );
            assert_eq!(
                verify([&retiring, &current], &headers, BODY, NOW),
                Ok(index)
            );
            assert!(verify([&retiring, &current], &headers, b"{}", NOW).is_err());
            headers.insert(ID_HEADER, "other".parse().unwrap());
            assert!(verify([&retiring, &current], &headers, BODY, NOW).is_err());
        }
    }

    #[test]
    fn malformed_headers_and_withdrawn_keys_are_refused() {
        let active = parse_secret(SECRET).unwrap();
        let retired = Webhook::from_bytes(vec![17; 32]).unwrap();
        let signed = sign(&retired, "msg", NOW, BODY).unwrap();
        let valid = headers(&signed);
        assert!(verify([&active], &valid, BODY, NOW).is_err());
        for name in [ID_HEADER, TIMESTAMP_HEADER, SIGNATURE_HEADER] {
            let mut incomplete = valid.clone();
            incomplete.remove(name);
            assert!(verify([&retired], &incomplete, BODY, NOW).is_err());
        }
        let mut unknown = valid.clone();
        unknown.insert(SIGNATURE_HEADER, "v2,unknown".parse().unwrap());
        assert!(verify([&retired], &unknown, BODY, NOW).is_err());
        let mut malformed = valid;
        malformed.insert(TIMESTAMP_HEADER, "yesterday".parse().unwrap());
        assert_eq!(
            verify([&retired], &malformed, BODY, NOW),
            Err(WebhookError::InvalidTimestamp)
        );
    }

    #[test]
    fn rejects_literal_keys_and_legacy_text_signatures() {
        for value in ["", "whsec_", "whsec_fleet-demo", "c2VjcmV0", "whsec_%%"] {
            assert!(parse_secret(value).is_err());
        }
        let key = parse_secret(SECRET).unwrap();
        // Padded Base64 encodings of 23, 24, 64 and 65 zero bytes.
        for (symbols, padding, accepted) in [
            (31, "=", false),
            (32, "", true),
            (86, "==", true),
            (87, "=", false),
        ] {
            let value = format!("whsec_{}{padding}", "A".repeat(symbols));
            assert_eq!(parse_secret(&value).is_ok(), accepted);
        }
        let legacy = Webhook::from_bytes(SECRET.as_bytes().to_vec()).unwrap();
        let headers = headers(&sign(&legacy, "msg", NOW, BODY).unwrap());
        assert!(verify([&key], &headers, BODY, NOW).is_err());
        assert!(verify([], &headers, BODY, NOW).is_err());
        assert!(sign(&key, "msg", NOW, &[0xff]).is_err());
    }
}
