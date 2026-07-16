use std::io::Cursor;

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder, ZlibDecoder};
use http::{HeaderMap, header::CONTENT_ENCODING};
use tokio::io::{AsyncRead, AsyncReadExt};

pub(super) const MAX_COMPRESSED_BYTES: usize = 2 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 10 * 1024 * 1024;
const MAX_EXPANSION_RATIO: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentEncoding {
    Identity,
    Gzip,
    Brotli,
    Deflate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodingError {
    Multiple,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidData,
    DecodedTooLarge,
    ExpansionRatio,
}

pub(super) fn content_encoding(headers: &HeaderMap) -> Result<ContentEncoding, EncodingError> {
    let mut values = headers.get_all(CONTENT_ENCODING).iter();
    let Some(value) = values.next() else {
        return Ok(ContentEncoding::Identity);
    };
    if values.next().is_some() {
        return Err(EncodingError::Multiple);
    }

    let bytes = trim_ows(value.as_bytes());
    if bytes.is_empty() || !bytes.is_ascii() || bytes.contains(&b',') || bytes.contains(&b';') {
        return Err(EncodingError::Invalid);
    }
    if bytes.eq_ignore_ascii_case(b"identity") {
        Ok(ContentEncoding::Identity)
    } else if bytes.eq_ignore_ascii_case(b"gzip") {
        Ok(ContentEncoding::Gzip)
    } else if bytes.eq_ignore_ascii_case(b"br") {
        Ok(ContentEncoding::Brotli)
    } else if bytes.eq_ignore_ascii_case(b"deflate") {
        Ok(ContentEncoding::Deflate)
    } else {
        Err(EncodingError::Invalid)
    }
}

pub(super) async fn decode_document(
    encoding: ContentEncoding,
    compressed: Vec<u8>,
) -> Result<Vec<u8>, DecodeError> {
    let compressed_len = compressed.len();
    match encoding {
        ContentEncoding::Identity => collect_decoded(Cursor::new(compressed), compressed_len).await,
        ContentEncoding::Gzip => {
            let mut decoder = GzipDecoder::new(Cursor::new(compressed));
            decoder.multiple_members(true);
            collect_decoded(decoder, compressed_len).await
        }
        ContentEncoding::Brotli => {
            collect_decoded(BrotliDecoder::new(Cursor::new(compressed)), compressed_len).await
        }
        ContentEncoding::Deflate => {
            collect_decoded(ZlibDecoder::new(Cursor::new(compressed)), compressed_len).await
        }
    }
}

pub(super) async fn collect_decoded<R>(
    mut reader: R,
    compressed_len: usize,
) -> Result<Vec<u8>, DecodeError>
where
    R: AsyncRead + Unpin,
{
    let ratio_limit = compressed_len.saturating_mul(MAX_EXPANSION_RATIO);
    let effective_limit = MAX_DECODED_BYTES.min(ratio_limit);
    let mut decoded = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let remaining = effective_limit.saturating_sub(decoded.len());
        let read_limit = buffer.len().min(remaining.saturating_add(1));
        let read = reader
            .read(&mut buffer[..read_limit])
            .await
            .map_err(|_| DecodeError::InvalidData)?;
        if read == 0 {
            return Ok(decoded);
        }
        let next_len = decoded
            .len()
            .checked_add(read)
            .ok_or(DecodeError::DecodedTooLarge)?;
        if next_len > MAX_DECODED_BYTES {
            return Err(DecodeError::DecodedTooLarge);
        }
        if next_len > ratio_limit {
            return Err(DecodeError::ExpansionRatio);
        }
        decoded.extend_from_slice(&buffer[..read]);
    }
}

fn trim_ows(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[1..];
    }
    while bytes
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use async_compression::tokio::write::{BrotliEncoder, GzipEncoder, ZlibEncoder};
    use http::{HeaderMap, HeaderValue, header::CONTENT_ENCODING};
    use tokio::io::AsyncWriteExt;

    use super::{
        ContentEncoding, DecodeError, EncodingError, MAX_DECODED_BYTES, collect_decoded,
        content_encoding, decode_document,
    };

    #[tokio::test]
    async fn compressed_and_decoded_limits_stop_streaming() {
        assert_eq!(
            collect_decoded(
                Cursor::new(vec![0_u8; MAX_DECODED_BYTES + 1]),
                MAX_DECODED_BYTES + 1,
            )
            .await
            .unwrap_err(),
            DecodeError::DecodedTooLarge
        );
        assert_eq!(
            collect_decoded(Cursor::new(vec![0_u8; 100]), 1)
                .await
                .unwrap()
                .len(),
            100
        );
        assert_eq!(
            collect_decoded(Cursor::new(vec![0_u8; 101]), 1)
                .await
                .unwrap_err(),
            DecodeError::ExpansionRatio
        );
    }

    #[tokio::test]
    async fn brotli_gzip_and_deflate_decode_within_budget() {
        let document = b"bounded feed document\n".repeat(32);
        assert_eq!(
            decode_document(ContentEncoding::Brotli, brotli(&document).await)
                .await
                .unwrap(),
            document
        );
        assert_eq!(
            decode_document(ContentEncoding::Gzip, gzip(&document).await)
                .await
                .unwrap(),
            document
        );
        let mut members = gzip(b"first-").await;
        members.extend(gzip(b"second").await);
        assert_eq!(
            decode_document(ContentEncoding::Gzip, members)
                .await
                .unwrap(),
            b"first-second"
        );
        assert_eq!(
            decode_document(ContentEncoding::Deflate, zlib(&document).await)
                .await
                .unwrap(),
            document
        );

        // Raw RFC 1951 DEFLATE for `abc`; HTTP `deflate` requires an RFC 1950
        // zlib wrapper and must reject this stream.
        let raw = vec![0x4b, 0x4c, 0x4a, 0x06, 0x00];
        assert_eq!(
            decode_document(ContentEncoding::Deflate, raw)
                .await
                .unwrap_err(),
            DecodeError::InvalidData
        );

        let mut headers = HeaderMap::new();
        assert_eq!(
            content_encoding(&headers).unwrap(),
            ContentEncoding::Identity
        );
        for (raw, expected) in [
            (" \tidentity\t ", ContentEncoding::Identity),
            ("GZIP", ContentEncoding::Gzip),
            ("Br", ContentEncoding::Brotli),
            ("DEFLATE", ContentEncoding::Deflate),
        ] {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_str(raw).unwrap());
            assert_eq!(content_encoding(&headers).unwrap(), expected);
        }
        for raw in ["", "gzip, br", "gzip;level=1", "compress"] {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_str(raw).unwrap());
            assert_eq!(content_encoding(&headers), Err(EncodingError::Invalid));
        }
        headers.insert(
            CONTENT_ENCODING,
            HeaderValue::from_bytes(b"gzip\xff").unwrap(),
        );
        assert_eq!(content_encoding(&headers), Err(EncodingError::Invalid));
        headers = HeaderMap::new();
        headers.append(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        headers.append(CONTENT_ENCODING, HeaderValue::from_static("br"));
        assert_eq!(content_encoding(&headers), Err(EncodingError::Multiple));
    }

    async fn brotli(input: &[u8]) -> Vec<u8> {
        let mut encoder = BrotliEncoder::new(Vec::new());
        encoder.write_all(input).await.unwrap();
        encoder.shutdown().await.unwrap();
        encoder.into_inner()
    }

    async fn gzip(input: &[u8]) -> Vec<u8> {
        let mut encoder = GzipEncoder::new(Vec::new());
        encoder.write_all(input).await.unwrap();
        encoder.shutdown().await.unwrap();
        encoder.into_inner()
    }

    async fn zlib(input: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new());
        encoder.write_all(input).await.unwrap();
        encoder.shutdown().await.unwrap();
        encoder.into_inner()
    }
}
