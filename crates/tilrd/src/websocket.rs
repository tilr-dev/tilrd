//! Outgoing WebSockets keep the bundled Mozilla trust store.

use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    Error, client::IntoClientRequest, handshake::client::Response,
};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

fn bundled_roots() -> RootCertStore {
    RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    }
}

fn connector(roots: RootCertStore) -> Connector {
    Connector::Rustls(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

pub async fn connect<R>(
    request: R,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect_with_roots(request, bundled_roots()).await
}

async fn connect_with_roots<R>(
    request: R,
    roots: RootCertStore,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector(roots)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn the_trust_store_is_exactly_the_bundled_mozilla_roots() {
        assert!(!bundled_roots().is_empty());
        assert_eq!(bundled_roots().roots, webpki_roots::TLS_SERVER_ROOTS);
    }

    #[test]
    fn websocket_dependencies_use_only_patched_bundled_roots() {
        let lock: toml::Value = toml::from_str(include_str!("../../../Cargo.lock")).unwrap();
        let versions = |name: &str| -> Vec<String> {
            lock["package"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["name"].as_str() == Some(name))
                .map(|p| p["version"].as_str().unwrap().to_owned())
                .collect()
        };
        let roots = versions("webpki-roots");
        assert!(!roots.is_empty());
        for version in roots {
            let parts: Vec<u64> = version.split('.').map(|s| s.parse().unwrap()).collect();
            assert!(
                parts.as_slice() >= [1, 0, 10].as_slice(),
                "affected roots: {version}"
            );
        }
        let verifiers = versions("rustls-webpki");
        assert!(!verifiers.is_empty());
        for version in verifiers {
            let parts: Vec<u64> = version.split('.').map(|s| s.parse().unwrap()).collect();
            assert!(
                parts.as_slice() >= [0, 103, 16].as_slice(),
                "affected verifier: {version}"
            );
        }
    }

    #[tokio::test]
    async fn plain_websockets_keep_request_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut request = format!("ws://{}", listener.local_addr().unwrap())
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer test".parse().unwrap());
        let server = async {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers()["authorization"], "Bearer test");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            ws.send(Message::text("connected")).await.unwrap();
        };
        let client = async {
            let (mut ws, _) = connect(request).await.unwrap();
            assert!(matches!(ws.get_ref(), MaybeTlsStream::Plain(_)));
            assert_eq!(
                ws.next().await.unwrap().unwrap(),
                Message::text("connected")
            );
        };
        tokio::join!(server, client);
    }

    fn test_identity() -> (CertificateDer<'static>, Arc<rustls::ServerConfig>) {
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate = identity.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], key.into())
            .unwrap();
        (certificate, Arc::new(config))
    }

    #[tokio::test]
    async fn an_explicit_root_allows_a_verified_secure_websocket() {
        let (certificate, config) = test_identity();
        let mut roots = RootCertStore::empty();
        roots.add(certificate).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = async {
            let (socket, _) = listener.accept().await.unwrap();
            let stream = tokio_rustls::TlsAcceptor::from(config)
                .accept(socket)
                .await
                .unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::text("verified")).await.unwrap();
        };
        let client = async {
            let (mut ws, _) = connect_with_roots(format!("wss://localhost:{port}"), roots)
                .await
                .unwrap();
            assert!(matches!(ws.get_ref(), MaybeTlsStream::Rustls(_)));
            assert_eq!(ws.next().await.unwrap().unwrap(), Message::text("verified"));
        };
        tokio::join!(server, client);
    }

    #[tokio::test]
    async fn bundled_roots_reject_an_untrusted_issuer() {
        let (_, config) = test_identity();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = async {
            let (socket, _) = listener.accept().await.unwrap();
            assert!(
                tokio_rustls::TlsAcceptor::from(config)
                    .accept(socket)
                    .await
                    .is_err()
            );
        };
        let client = async {
            let err = connect(format!("wss://localhost:{port}"))
                .await
                .unwrap_err();
            assert_certificate_error(err, |err| {
                matches!(err, rustls::CertificateError::UnknownIssuer)
            });
        };
        tokio::join!(server, client);
    }

    #[tokio::test]
    async fn an_explicit_root_does_not_allow_a_wrong_hostname() {
        let (certificate, config) = test_identity();
        let mut roots = RootCertStore::empty();
        roots.add(certificate).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (socket, _) = listener.accept().await.unwrap();
            assert!(
                tokio_rustls::TlsAcceptor::from(config)
                    .accept(socket)
                    .await
                    .is_err()
            );
        };
        let client = async {
            let err = connect_with_roots(format!("wss://{address}"), roots)
                .await
                .unwrap_err();
            assert_certificate_error(err, |err| {
                matches!(
                    err,
                    rustls::CertificateError::NotValidForName
                        | rustls::CertificateError::NotValidForNameContext { .. }
                )
            });
        };
        tokio::join!(server, client);
    }

    fn assert_certificate_error(
        err: Error,
        expected: impl FnOnce(&rustls::CertificateError) -> bool,
    ) {
        let Error::Io(err) = err else {
            panic!("expected TLS I/O error, got {err:?}");
        };
        let Some(rustls::Error::InvalidCertificate(certificate_error)) = err
            .get_ref()
            .and_then(|err| err.downcast_ref::<rustls::Error>())
        else {
            panic!("expected certificate error, got {err:?}");
        };
        assert!(
            expected(certificate_error),
            "unexpected certificate error: {certificate_error:?}"
        );
    }
}
