//! Web Push on the wire: message encryption and the box's signature.
//!
//! Two RFCs and nothing else. `[RFC 8291]` encrypts a message so that the push
//! service relaying it — Apple's, Google's, Mozilla's — carries ciphertext and
//! learns only that this box sent this browser something. `[RFC 8292]` (VAPID)
//! signs the request with a key the browser was given when it subscribed, so
//! nobody else can send to the subscription.
//!
//! Pure functions over bytes, on the provider the rest of the box already links
//! (`aws-lc-rs`, D84): no socket, no clock of its own, no store. The encryption
//! is held to the RFC's own worked example, intermediate values included.

use aws_lc_rs::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, ECDH_P256, PrivateKey, UnparsedPublicKey};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use aws_lc_rs::{hmac, rand};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// An uncompressed P-256 point: `0x04`, X, Y.
pub const POINT: usize = 65;
/// A subscription's authentication secret — `[RFC 8291 3.2]`.
pub const AUTH: usize = 16;
/// The most plaintext one message carries.
///
/// A push service must accept 4096 octets of body `[RFC 8030 7.2]`; what is left
/// after the 86-octet header, the padding delimiter and the tag is 3993. A box
/// that sends a sentence is nowhere near it, and one that is has put something
/// in the payload that does not belong there.
pub const MAX_PLAINTEXT: usize = 3993;

/// What can go wrong between a message and its ciphertext.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The subscription's public key is not a P-256 point.
    #[error("the subscription's key is not an uncompressed P-256 point")]
    Key,
    /// The subscription's secret is not sixteen octets.
    #[error("the subscription's authentication secret is not 16 octets")]
    Auth,
    /// More than [`MAX_PLAINTEXT`].
    #[error("the message is longer than one push record")]
    TooLong,
    /// The provider refused — entropy, or a key it would not take.
    #[error("the cryptographic provider refused")]
    Provider,
    /// The stored signing key cannot be read.
    #[error("the stored signing key is not a P-256 PKCS#8 document")]
    SigningKey,
}

/// base64url without padding, the only spelling Web Push uses.
#[must_use]
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Read base64url. Padding is tolerated, because browsers disagree about it.
#[must_use]
pub fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// One step of HKDF-SHA-256 with a single output block — `[RFC 5869]`.
///
/// Everything `[RFC 8291]` derives is at most 32 octets, so the expand step is
/// one HMAC and a counter of one.
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, salt), ikm);
    let key = hmac::Key::new(hmac::HMAC_SHA256, prk.as_ref());
    let mut ctx = hmac::Context::with_key(&key);
    ctx.update(info);
    ctx.update(&[1]);
    ctx.sign().as_ref()[..len].to_vec()
}

/// The content-encryption key and nonce — `[RFC 8291 3.3, 3.4]`.
fn derive(
    ecdh: &[u8],
    auth: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8],
) -> ([u8; 16], [u8; 12]) {
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let ikm = hkdf(auth, ecdh, &key_info, 32);
    let mut cek = [0; 16];
    cek.copy_from_slice(&hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16));
    let mut nonce = [0; 12];
    nonce.copy_from_slice(&hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12));
    (cek, nonce)
}

/// Encrypt `plaintext` for one subscription — `[RFC 8291]`, `aes128gcm`.
///
/// A fresh key pair and salt for every message, which is what makes two
/// messages to one browser unlinkable by their bytes.
///
/// # Errors
/// [`Error`] for a subscription whose keys are malformed, a message too long
/// for one record, or a provider that refused.
pub fn encrypt(plaintext: &[u8], ua_public: &[u8], auth: &[u8]) -> Result<Vec<u8>, Error> {
    let sender = PrivateKey::generate(&ECDH_P256).map_err(|_| Error::Provider)?;
    let mut salt = [0; 16];
    rand::fill(&mut salt).map_err(|_| Error::Provider)?;
    seal(plaintext, ua_public, auth, &sender, salt)
}

/// [`encrypt`] with the sender's key and the salt given, so a test can hold the
/// result to the RFC's example.
fn seal(
    plaintext: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    sender: &PrivateKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, Error> {
    if ua_public.len() != POINT || ua_public[0] != 4 {
        return Err(Error::Key);
    }
    if auth.len() != AUTH {
        return Err(Error::Auth);
    }
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::TooLong);
    }
    let as_public = sender.compute_public_key().map_err(|_| Error::Provider)?;
    let as_public = as_public.as_ref();
    let (cek, nonce) = agreement::agree(
        sender,
        UnparsedPublicKey::new(&ECDH_P256, ua_public),
        Error::Key,
        |ecdh| Ok(derive(ecdh, auth, ua_public, as_public, &salt)),
    )?;

    // `[RFC 8188 2.1]`: salt, record size, key id length, key id — then the one
    // record, whose plaintext ends in the last-record delimiter `0x02`.
    let mut body = Vec::with_capacity(86 + plaintext.len() + 17);
    body.extend_from_slice(&salt);
    body.extend_from_slice(&4096_u32.to_be_bytes());
    body.push(u8::try_from(POINT).map_err(|_| Error::Provider)?);
    body.extend_from_slice(as_public);
    let mut record = plaintext.to_vec();
    record.push(2);
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).map_err(|_| Error::Provider)?);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut record,
    )
    .map_err(|_| Error::Provider)?;
    body.extend_from_slice(&record);
    Ok(body)
}

/// An ES256 signature over `input` by the P-256 key in the PKCS#8 `document`:
/// the fixed-width `r ‖ s` a JWS carries, `[RFC 7518 3.4]`.
///
/// # Errors
/// [`Error::SigningKey`] where the document is not a P-256 key, and
/// [`Error::Provider`] where signing fails.
pub fn sign_es256(document: &[u8], input: &[u8]) -> Result<Vec<u8>, Error> {
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document)
        .map_err(|_| Error::SigningKey)?;
    pair.sign(&rand::SystemRandom::new(), input)
        .map(|signature| signature.as_ref().to_vec())
        .map_err(|_| Error::Provider)
}

/// The box's Web Push signing key — `[RFC 8292]`.
pub struct Vapid {
    pair: EcdsaKeyPair,
}

impl Vapid {
    /// A new key, as the PKCS#8 document the store keeps.
    ///
    /// # Errors
    /// [`Error::Provider`] without entropy.
    pub fn generate() -> Result<Vec<u8>, Error> {
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rand::SystemRandom::new())
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| Error::Provider)
    }

    /// The key a stored document holds.
    ///
    /// # Errors
    /// [`Error::SigningKey`] where the document is not one.
    pub fn from_pkcs8(document: &[u8]) -> Result<Self, Error> {
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document)
            .map(|pair| Self { pair })
            .map_err(|_| Error::SigningKey)
    }

    /// The public key a browser subscribes with (`applicationServerKey`),
    /// base64url.
    #[must_use]
    pub fn public_key(&self) -> String {
        b64(self.pair.public_key().as_ref())
    }

    /// The `Authorization` header for one request to `audience` — the push
    /// service's origin — valid until `expires` (Unix seconds).
    ///
    /// `[RFC 8292 2]`: a JWT signed ES256 carrying `aud`, `exp` (at most a day
    /// ahead) and a contact in `sub`.
    ///
    /// # Errors
    /// [`Error::Provider`] where signing fails.
    pub fn authorization(
        &self,
        audience: &str,
        expires: i64,
        contact: &str,
    ) -> Result<String, Error> {
        let claims = serde_json::json!({ "aud": audience, "exp": expires, "sub": contact });
        let signed = format!(
            "{}.{}",
            b64(br#"{"typ":"JWT","alg":"ES256"}"#),
            b64(claims.to_string().as_bytes())
        );
        let signature = self
            .pair
            .sign(&rand::SystemRandom::new(), signed.as_bytes())
            .map_err(|_| Error::Provider)?;
        Ok(format!(
            "vapid t={signed}.{}, k={}",
            b64(signature.as_ref()),
            self.public_key()
        ))
    }
}

/// A browser's half, for tests: a key pair, a secret and the way back.
#[cfg(test)]
pub mod receiver {
    use super::*;

    /// What a browser keeps for one subscription.
    pub struct Receiver {
        private: PrivateKey,
        /// `p256dh`, uncompressed.
        pub public: Vec<u8>,
        /// The authentication secret.
        pub auth: [u8; AUTH],
    }

    impl Receiver {
        /// A new subscription's keys.
        ///
        /// # Panics
        /// Without entropy.
        #[must_use]
        pub fn fresh() -> Self {
            let private = PrivateKey::generate(&ECDH_P256).expect("a key");
            let mut auth = [0; AUTH];
            rand::fill(&mut auth).expect("entropy");
            Self::of(private, auth)
        }

        /// One with a given key and secret.
        ///
        /// # Panics
        /// Where the key has no public point.
        #[must_use]
        pub fn of(private: PrivateKey, auth: [u8; AUTH]) -> Self {
            let public = private
                .compute_public_key()
                .expect("a point")
                .as_ref()
                .to_vec();
            Self {
                private,
                public,
                auth,
            }
        }

        /// Read a message the way a browser does — `[RFC 8291 4]`.
        #[must_use]
        pub fn open(&self, body: &[u8]) -> Option<Vec<u8>> {
            let (salt, rest) = body.split_at_checked(16)?;
            let (_record_size, rest) = rest.split_at_checked(4)?;
            let (&id_len, rest) = rest.split_first()?;
            let (as_public, record) = rest.split_at_checked(usize::from(id_len))?;
            let (cek, nonce) = agreement::agree(
                &self.private,
                UnparsedPublicKey::new(&ECDH_P256, as_public),
                (),
                |ecdh| Ok(derive(ecdh, &self.auth, &self.public, as_public, salt)),
            )
            .ok()?;
            let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).ok()?);
            let mut record = record.to_vec();
            let plain = key
                .open_in_place(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::empty(),
                    &mut record,
                )
                .ok()?;
            let (&delimiter, message) = plain.split_last()?;
            (delimiter == 2).then(|| message.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::receiver::Receiver;
    use super::*;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as Verifier};

    // `[RFC 8291 5]` and its Appendix A: the one example the standard works.
    const PLAINTEXT: &[u8] = b"When I grow up, I want to be a watermelon";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml\
                        mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT\
                        pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    fn private(text: &str) -> PrivateKey {
        PrivateKey::from_private_key(&ECDH_P256, &unb64(text).expect("base64url")).expect("a key")
    }

    #[test]
    fn the_rfc_s_own_example_encrypts_to_the_rfc_s_own_bytes() {
        let salt: [u8; 16] = unb64(SALT).unwrap().try_into().unwrap();
        let body = seal(
            PLAINTEXT,
            &unb64(UA_PUBLIC).unwrap(),
            &unb64(AUTH_SECRET).unwrap(),
            &private(AS_PRIVATE),
            salt,
        )
        .expect("a message");
        assert_eq!(b64(&body), BODY, "[RFC 8291 5], octet for octet");
    }

    #[test]
    fn the_rfc_s_intermediate_values_are_reproduced() {
        // Appendix A: the key and nonce, from the shared secret it states.
        let (cek, nonce) = derive(
            &unb64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs").unwrap(),
            &unb64(AUTH_SECRET).unwrap(),
            &unb64(UA_PUBLIC).unwrap(),
            private(AS_PRIVATE).compute_public_key().unwrap().as_ref(),
            &unb64(SALT).unwrap(),
        );
        assert_eq!(b64(&cek), "oIhVW04MRdy2XN9CiKLxTg");
        assert_eq!(b64(&nonce), "4h_95klXJ5E_qnoN");
    }

    #[test]
    fn the_receiver_reads_the_rfc_s_message_and_a_fresh_one() {
        let auth: [u8; AUTH] = unb64(AUTH_SECRET).unwrap().try_into().unwrap();
        let receiver = Receiver::of(private(UA_PRIVATE), auth);
        assert_eq!(
            receiver.open(&unb64(BODY).unwrap()).as_deref(),
            Some(PLAINTEXT)
        );

        let fresh = Receiver::fresh();
        let first = encrypt(b"hello", &fresh.public, &fresh.auth).unwrap();
        let second = encrypt(b"hello", &fresh.public, &fresh.auth).unwrap();
        assert_ne!(first, second, "a new key and salt for every message");
        assert_eq!(fresh.open(&first).as_deref(), Some(&b"hello"[..]));
        assert_eq!(
            receiver.open(&first),
            None,
            "another subscription cannot read it"
        );
    }

    #[test]
    fn malformed_subscriptions_and_oversized_messages_are_refused() {
        let r = Receiver::fresh();
        assert_eq!(encrypt(b"x", &r.public[..64], &r.auth), Err(Error::Key));
        let mut compressed = r.public.clone();
        compressed[0] = 2;
        assert_eq!(encrypt(b"x", &compressed, &r.auth), Err(Error::Key));
        // On the curve is the provider's check, not a length check.
        let mut off_curve = r.public.clone();
        off_curve[64] ^= 1;
        assert_eq!(encrypt(b"x", &off_curve, &r.auth), Err(Error::Key));
        assert_eq!(encrypt(b"x", &r.public, &r.auth[..15]), Err(Error::Auth));
        assert_eq!(
            encrypt(&vec![0; MAX_PLAINTEXT + 1], &r.public, &r.auth),
            Err(Error::TooLong)
        );
        let full = encrypt(&vec![0; MAX_PLAINTEXT], &r.public, &r.auth).unwrap();
        assert_eq!(
            full.len(),
            4096,
            "[RFC 8030 7.2]: exactly what a service must accept"
        );
    }

    #[test]
    fn the_signature_is_a_jwt_the_public_key_verifies() {
        let document = Vapid::generate().unwrap();
        let vapid = Vapid::from_pkcs8(&document).unwrap();
        let again = Vapid::from_pkcs8(&document).unwrap();
        assert_eq!(vapid.public_key(), again.public_key(), "kept, not re-made");
        let public = unb64(&vapid.public_key()).unwrap();
        assert_eq!((public.len(), public[0]), (POINT, 4));

        let header = vapid
            .authorization(
                "https://push.example.net",
                1_800_000_000,
                "mailto:box@example.net",
            )
            .unwrap();
        let rest = header.strip_prefix("vapid t=").expect("[RFC 8292 3]");
        let (token, key) = rest.split_once(", k=").expect("both parameters");
        assert_eq!(key, vapid.public_key());
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let head: serde_json::Value = serde_json::from_slice(&unb64(parts[0]).unwrap()).unwrap();
        assert_eq!(head, serde_json::json!({"typ": "JWT", "alg": "ES256"}));
        let claims: serde_json::Value = serde_json::from_slice(&unb64(parts[1]).unwrap()).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({
                "aud": "https://push.example.net",
                "exp": 1_800_000_000,
                "sub": "mailto:box@example.net"
            })
        );
        let signature = unb64(parts[2]).unwrap();
        assert_eq!(signature.len(), 64, "ES256 is r‖s, not DER");
        Verifier::new(&ECDSA_P256_SHA256_FIXED, &public)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .expect("the browser's key verifies the box's signature");
        assert!(Vapid::from_pkcs8(b"not a key").is_err());
    }
}
