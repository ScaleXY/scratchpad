//! Self-signed certificate for the optional SMTP STARTTLS extension.

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;

/// Installs `ring` as the process-wide rustls crypto provider (idempotent).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn self_signed_acceptor() -> anyhow::Result<TlsAcceptor> {
    install_crypto_provider();
    let names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "scratchpad.local".to_string(),
    ];
    let certified = rcgen::generate_simple_self_signed(names)?;
    let cert = CertificateDer::from(certified.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Client config trusting the Mozilla root store (used for IMAP remote mailboxes).
pub fn client_config() -> Arc<rustls::ClientConfig> {
    install_crypto_provider();
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}
