use rustls::compress::{
    BROTLI_COMPRESSOR, BROTLI_DECOMPRESSOR, CertCompressor, CertDecompressor, CompressionLevel,
    ZLIB_COMPRESSOR, ZLIB_DECOMPRESSOR,
};

#[test]
fn certificate_brotli_rejects_large_window_format() {
    let large_window_empty = [0x11, 0xd0];

    assert!(
        BROTLI_DECOMPRESSOR
            .decompress(&large_window_empty, &mut [])
            .is_err(),
        "certificate compression must reject the large-window Brotli extension"
    );
}

#[test]
fn certificate_codecs_roundtrip_without_accepting_trailing_data() {
    let codecs: [(&dyn CertCompressor, &dyn CertDecompressor); 2] = [
        (BROTLI_COMPRESSOR, BROTLI_DECOMPRESSOR),
        (ZLIB_COMPRESSOR, ZLIB_DECOMPRESSOR),
    ];
    let original = b"bounded certificate compression fixture".repeat(64);

    for (compressor, decompressor) in codecs {
        let mut compressed = compressor
            .compress(original.clone(), CompressionLevel::Interactive)
            .unwrap();
        let mut recovered = vec![0; original.len()];
        decompressor
            .decompress(&compressed, &mut recovered)
            .unwrap();
        assert_eq!(recovered, original, "{:?}", decompressor.algorithm());

        compressed.extend_from_slice(b"trailing data");
        assert!(
            decompressor
                .decompress(&compressed, &mut recovered)
                .is_err(),
            "{:?} must consume the complete compressed message",
            decompressor.algorithm()
        );
    }
}
