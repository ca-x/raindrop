use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::crypto::ring::{cipher_suite, kx_group};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, Error, PeerMisbehaved, RootCertStore};

#[test]
fn tls13_rejects_plaintext_encrypted_extensions_after_server_hello() {
    let mut client = tls13_client();
    let mut handshake = server_hello(&mut client);
    handshake.extend_from_slice(&[8, 0, 0, 2, 0, 0]);
    let record = plaintext_handshake_record(&handshake);

    assert_eq!(
        client.read_tls(&mut record.as_slice()).unwrap(),
        record.len()
    );
    assert_eq!(
        client.process_new_packets().unwrap_err(),
        Error::PeerMisbehaved(PeerMisbehaved::KeyEpochWithPendingFragment)
    );
}

#[test]
fn tls13_accepts_server_hello_without_cross_epoch_messages() {
    let mut client = tls13_client();
    let record = plaintext_handshake_record(&server_hello(&mut client));

    assert_eq!(
        client.read_tls(&mut record.as_slice()).unwrap(),
        record.len()
    );
    client.process_new_packets().unwrap();
    assert!(client.is_handshaking());
}

fn tls13_client() -> ClientConnection {
    let provider = CryptoProvider {
        cipher_suites: vec![cipher_suite::TLS13_AES_128_GCM_SHA256],
        kx_groups: vec![kx_group::SECP256R1],
        ..rustls::crypto::ring::default_provider()
    };
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();

    ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("example.test").unwrap(),
    )
    .unwrap()
}

fn server_hello(client: &mut ClientConnection) -> Vec<u8> {
    let mut client_hello = Vec::new();
    client.write_tls(&mut client_hello).unwrap();
    assert_eq!(client_hello[0], 22);
    assert_eq!(client_hello[5], 1);
    let session_id_length = client_hello[43];
    let session_id = &client_hello[44..44 + usize::from(session_id_length)];
    let server_exchange = kx_group::SECP256R1.start().unwrap();
    let public_key = server_exchange.pub_key();
    let public_key_length = u16::try_from(public_key.len()).unwrap();

    let mut extensions = vec![0, 43, 0, 2, 3, 4, 0, 51];
    extensions.extend_from_slice(&(public_key_length + 4).to_be_bytes());
    extensions.extend_from_slice(&[0, 23]);
    extensions.extend_from_slice(&public_key_length.to_be_bytes());
    extensions.extend_from_slice(public_key);

    let mut body = vec![3, 3];
    body.extend_from_slice(&[42; 32]);
    body.push(session_id_length);
    body.extend_from_slice(session_id);
    body.extend_from_slice(&[0x13, 0x01, 0]);
    body.extend_from_slice(&u16::try_from(extensions.len()).unwrap().to_be_bytes());
    body.extend_from_slice(&extensions);

    let mut handshake = vec![2];
    handshake.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes()[1..]);
    handshake.extend_from_slice(&body);
    handshake
}

fn plaintext_handshake_record(handshake: &[u8]) -> Vec<u8> {
    let mut record = vec![22, 3, 3];
    record.extend_from_slice(&u16::try_from(handshake.len()).unwrap().to_be_bytes());
    record.extend_from_slice(handshake);
    record
}
