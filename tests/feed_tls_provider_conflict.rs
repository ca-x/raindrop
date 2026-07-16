use raindrop::feeds::{CryptoProviderError, install_ring_crypto_provider};

#[test]
fn conflicting_process_crypto_provider_is_rejected() {
    let mut conflicting = rustls::crypto::ring::default_provider();
    conflicting.cipher_suites.clear();
    assert!(
        conflicting.install_default().is_ok(),
        "integration test process must start without a crypto provider"
    );

    assert_eq!(install_ring_crypto_provider(), Err(CryptoProviderError));
}
