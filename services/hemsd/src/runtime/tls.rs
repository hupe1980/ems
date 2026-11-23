//! The box's own HTTPS.
//!
//! A household gateway is on a network nobody secures: phones, a television, a
//! guest's laptop. A token sent in clear there is a token any of them can read,
//! and an S2 manager will not send one at all — S2 Connect forbids S2 over an
//! unencrypted channel, and `s2-kit` refuses to. So the box serves HTTPS with a
//! certificate it issues itself and **keeps**, for the reason it keeps its SHIP
//! identity (D215): the fingerprint is what a browser or a manager pins at
//! commissioning, and one that changed on every start would be a stranger
//! after every reboot.
//!
//! Self-signed by necessity. No public CA will sign `hems.local`, and a box
//! that needed one would need the WAN to be reached on its own LAN. The
//! fingerprint is printed at start-up, shown on the household page and on the
//! handover sheet, which is how it is checked.

use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// The names the certificate answers to on a household network.
const NAMES: [&str; 2] = ["hems.local", "localhost"];

/// The TLS configuration the box serves with, and its certificate's SHA-256
/// fingerprint.
///
/// # Errors
/// Where the stored certificate cannot be read or a new one cannot be made.
pub fn server_config(
    store: Option<&crate::store::Store>,
) -> anyhow::Result<(Arc<rustls::ServerConfig>, String)> {
    let kept = store
        .map(crate::store::Store::api_tls)
        .transpose()?
        .flatten();
    let (certificate_pem, key_pem) = if let Some(kept) = kept {
        kept
    } else {
        {
            let made = issue()?;
            if let Some(store) = store {
                store.put_api_tls(&made.0, &made.1)?;
            } else {
                tracing::warn!(
                    "no store is configured, so this box's HTTPS certificate is new on every \
                     start and anything that pinned it will have to pin it again"
                );
            }
            made
        }
    };
    let certificate = CertificateDer::from_pem_slice(certificate_pem.as_bytes())?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())?;
    let fingerprint = eebus::ship::Fingerprint::of_der(certificate.as_ref()).to_hex();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok((Arc::new(config), fingerprint))
}

/// A new self-signed P-256 certificate for [`NAMES`], as PEM.
fn issue() -> anyhow::Result<(String, String)> {
    let key = rcgen::KeyPair::generate()?;
    let params =
        rcgen::CertificateParams::new(NAMES.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())?;
    let certificate = params.self_signed(&key)?;
    Ok((certificate.pem(), key.serialize_pem()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_serves_the_same_certificate() {
        let store = crate::store::Store::in_memory().expect("a store");
        let (_, first) = server_config(Some(&store)).expect("a certificate");
        let (_, second) = server_config(Some(&store)).expect("the same one");
        assert_eq!(
            first, second,
            "the fingerprint a browser pinned survives a restart"
        );
        assert_eq!(first.len(), 64, "SHA-256, hex");
    }

    #[test]
    fn factory_reset_rotates_the_https_identity() {
        let mut store = crate::store::Store::in_memory().unwrap();
        let (_, old) = server_config(Some(&store)).unwrap();
        store.factory_reset().unwrap();
        let (_, new) = server_config(Some(&store)).unwrap();
        assert_ne!(old, new);
    }

    #[test]
    fn a_lan_address_in_clear_is_refused_unless_written_down() {
        let lan: std::net::SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let loopback: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let mut api = crate::runtime::access::ApiSettings::default();
        assert_eq!(api.tls_on(lan), Ok(true), "on by default off loopback");
        assert_eq!(api.tls_on(loopback), Ok(false));
        api.tls = Some(false);
        assert!(api.tls_on(lan).is_err(), "clear on the LAN is refused");
        api.insecure_lan = true;
        assert_eq!(api.tls_on(lan), Ok(false), "unless written down");
    }
}
