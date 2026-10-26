//! Loopback peer whose issuer name matches a bundled root, but whose key does not.
use std::sync::Arc;

use rcgen::{CertificateParams, DistinguishedName, DnType, DnValue, Issuer, KeyPair};
use rustls::{RootCertStore, pki_types::PrivatePkcs8KeyDer};
use tokio::{net::TcpListener, task::JoinHandle};

pub async fn peer_with_bundled_issuer_name() -> (String, JoinHandle<()>) {
    let mut ca = CertificateParams::default();
    ca.distinguished_name = DistinguishedName::new();
    for (kind, value) in [
        (DnType::CountryName, "US"),
        (DnType::OrganizationName, "Internet Security Research Group"),
        (DnType::CommonName, "ISRG Root X2"),
    ] {
        ca.distinguished_name
            .push(kind, DnValue::PrintableString(value.try_into().unwrap()));
    }
    // Match the real anchor's algorithm, but deliberately generate a different key.
    let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let mut probe = RootCertStore::empty();
    probe.add(ca_cert.der().clone()).unwrap();
    assert!(webpki_roots::TLS_SERVER_ROOTS.iter().any(|root| {
        root.subject == probe.roots[0].subject
            && root.subject_public_key_info != probe.roots[0].subject_public_key_info
    }));
    let issuer = Issuer::new(ca, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        assert!(
            tokio_rustls::TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await
                .is_err()
        );
    });
    (url, server)
}

pub fn assert_bundled_root_was_used(error: impl std::fmt::Debug) {
    // An empty store (including connect_async's implicit store) yields UnknownIssuer.
    // BadSignature proves verification reached the matching bundled trust anchor.
    let error = format!("{error:?}");
    assert!(
        error.contains("BadSignature"),
        "expected bundled anchor signature failure, got {error}"
    );
}
