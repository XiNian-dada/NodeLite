//! Agent sessions and upgrade probes share current Mozilla roots and strict TLS verification.

use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    Error as WebSocketError, handshake::client::Response, protocol::WebSocketConfig,
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};

#[derive(Debug, Error)]
pub(crate) enum ConnectError {
    #[error("failed to configure TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error(transparent)]
    WebSocket(#[from] WebSocketError),
}

pub(crate) async fn connect(
    server: &str,
    config: WebSocketConfig,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), ConnectError> {
    let connector = Connector::Rustls(Arc::new(client_config(current_roots())?));
    Ok(connect_async_tls_with_config(server, Some(config), false, Some(connector)).await?)
}

fn current_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

fn client_config(roots: RootCertStore) -> Result<ClientConfig, rustls::Error> {
    // Workspace feature unification may enable another provider; keep Agent TLS on ring.
    Ok(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use openssl::asn1::Asn1Time;
    use openssl::bn::BigNum;
    use openssl::hash::MessageDigest;
    use openssl::pkey::{PKey, Private};
    use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectAlternativeName};
    use openssl::x509::{X509, X509NameBuilder};
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
    use rustls::{ClientConnection, ServerConfig, ServerConnection};
    use tokio::net::TcpListener;

    use super::*;

    struct Fixture {
        root: X509,
        certificate: X509,
        key: PKey<Private>,
    }

    fn fixture() -> Fixture {
        let root_key = PKey::generate_ed25519().expect("generate fixture CA key");
        let mut name = X509NameBuilder::new().expect("CA name");
        name.append_entry_by_text("CN", "NodeLite ephemeral test CA")
            .expect("CA subject");
        let name = name.build();
        let mut root = X509::builder().expect("CA builder");
        root.set_version(2).expect("CA version");
        let serial = BigNum::from_u32(1)
            .expect("CA serial")
            .to_asn1_integer()
            .expect("CA serial");
        root.set_serial_number(&serial).expect("CA serial");
        root.set_subject_name(&name).expect("CA subject");
        root.set_issuer_name(&name).expect("CA issuer");
        root.set_pubkey(&root_key).expect("CA public key");
        root.set_not_before(&Asn1Time::days_from_now(0).expect("CA start"))
            .expect("CA start");
        root.set_not_after(&Asn1Time::days_from_now(1).expect("CA expiry"))
            .expect("CA expiry");
        root.append_extension(
            BasicConstraints::new()
                .critical()
                .ca()
                .build()
                .expect("CA constraints"),
        )
        .expect("CA constraints");
        root.append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .build()
                .expect("CA usage"),
        )
        .expect("CA usage");
        root.sign(&root_key, MessageDigest::null())
            .expect("sign CA");
        let root = root.build();
        let key = PKey::generate_ed25519().expect("generate fixture server key");
        let mut certificate = X509::builder().expect("server certificate builder");
        certificate
            .set_version(2)
            .expect("server certificate version");
        let serial = BigNum::from_u32(2)
            .expect("server serial")
            .to_asn1_integer()
            .expect("server serial");
        certificate
            .set_serial_number(&serial)
            .expect("server serial");
        let mut server_name = X509NameBuilder::new().expect("server name");
        server_name
            .append_entry_by_text("CN", "localhost")
            .expect("server subject");
        certificate
            .set_subject_name(&server_name.build())
            .expect("server subject");
        certificate.set_issuer_name(&name).expect("server issuer");
        certificate.set_pubkey(&key).expect("server public key");
        certificate
            .set_not_before(&Asn1Time::days_from_now(0).expect("server start"))
            .expect("server start");
        certificate
            .set_not_after(&Asn1Time::days_from_now(1).expect("server expiry"))
            .expect("server expiry");
        let san = SubjectAlternativeName::new()
            .dns("localhost")
            .build(&certificate.x509v3_context(Some(&root), None))
            .expect("server SAN");
        certificate.append_extension(san).expect("server SAN");
        certificate
            .sign(&root_key, MessageDigest::null())
            .expect("sign server certificate");
        Fixture {
            root,
            certificate: certificate.build(),
            key,
        }
    }

    fn trusted_config(fixture: &Fixture) -> ClientConfig {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(fixture.root.to_der().expect("CA DER")))
            .expect("trust fixture CA");
        client_config(roots).expect("trusted client configuration")
    }

    fn handshake(config: ClientConfig, fixture: &Fixture, host: &str) -> Result<(), rustls::Error> {
        let server =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("TLS versions")
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(
                        fixture.certificate.to_der().expect("server DER"),
                    )],
                    PrivatePkcs8KeyDer::from(
                        fixture.key.private_key_to_pkcs8().expect("server key DER"),
                    )
                    .into(),
                )
                .expect("server configuration");
        let mut server = ServerConnection::new(Arc::new(server)).expect("server connection");
        let mut client = ClientConnection::new(
            Arc::new(config),
            ServerName::try_from(host.to_string()).expect("fixture hostname"),
        )
        .expect("client connection");
        for _ in 0..8 {
            let mut outgoing = Vec::new();
            client.write_tls(&mut outgoing).expect("client TLS output");
            server
                .read_tls(&mut Cursor::new(outgoing))
                .expect("server TLS input");
            server.process_new_packets()?;
            let mut outgoing = Vec::new();
            server.write_tls(&mut outgoing).expect("server TLS output");
            client
                .read_tls(&mut Cursor::new(outgoing))
                .expect("client TLS input");
            client.process_new_packets()?;
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
        }
        panic!("in-memory TLS handshake did not finish");
    }

    #[test]
    fn trusted_certificate_completes_tls_handshake() {
        let fixture = fixture();
        handshake(trusted_config(&fixture), &fixture, "localhost").expect("trusted TLS handshake");
    }

    #[test]
    fn public_roots_reject_an_unknown_certificate_authority() {
        let error = handshake(
            client_config(current_roots()).expect("public TLS config"),
            &fixture(),
            "localhost",
        )
        .expect_err("unknown CA must fail");
        assert!(matches!(error, rustls::Error::InvalidCertificate(_)));
    }

    #[test]
    fn trusted_certificate_for_another_hostname_is_rejected() {
        let fixture = fixture();
        let error = handshake(trusted_config(&fixture), &fixture, "other.example")
            .expect_err("wrong hostname must fail");
        assert!(matches!(error, rustls::Error::InvalidCertificate(_)));
    }

    #[test]
    fn current_roots_exclude_the_distrusted_emsign_ecc_anchor() {
        let roots = current_roots();
        assert!(!roots.is_empty());
        let distrusted = b"emSign ECC Root CA - G3";
        assert!(roots.roots.iter().all(|root| {
            !root
                .subject
                .as_ref()
                .windows(distrusted.len())
                .any(|part| part == distrusted)
        }));
    }

    #[tokio::test]
    async fn explicit_tls_connector_preserves_plain_loopback_websockets() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let address = listener.local_addr().expect("fixture address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("fixture accept");
            tokio_tungstenite::accept_async(stream)
                .await
                .expect("fixture handshake");
        });
        let connection = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connect(&format!("ws://{address}"), WebSocketConfig::default()),
        )
        .await
        .expect("connection deadline")
        .expect("plain websocket compatibility");
        drop(connection);
        server.await.expect("fixture server task");
    }
}
