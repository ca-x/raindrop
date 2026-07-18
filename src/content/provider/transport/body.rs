use std::{io::Cursor, time::Duration};

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder, ZlibDecoder};
use http::{
    HeaderMap,
    header::{CONTENT_ENCODING, CONTENT_LENGTH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::Instant,
};

use super::{
    ProviderTimeoutStage, ProviderTransportError, ProviderTransportErrorKind,
    http::{BodyError, HttpBody},
    strict_timeout_at,
};

const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_COMPRESSED_BYTES: usize = 2 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPANSION_RATIO: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentEncoding {
    Identity,
    Gzip,
    Brotli,
    Deflate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecodeError {
    InvalidData,
    TooLarge,
}

pub(super) async fn collect_response_body(
    provider_id: &str,
    headers: &HeaderMap,
    body: &mut dyn HttpBody,
    total_deadline: Instant,
) -> Result<Vec<u8>, ProviderTransportError> {
    validate_content_length(provider_id, headers)?;
    let encoding = content_encoding(provider_id, headers)?;
    let compressed = collect_compressed(provider_id, body, total_deadline).await?;
    strict_timeout_at(total_deadline, decode_response(encoding, compressed))
        .await
        .map_err(|_| ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::Total))?
        .map_err(|error| match error {
            DecodeError::InvalidData => {
                ProviderTransportError::new(provider_id, ProviderTransportErrorKind::Decode)
            }
            DecodeError::TooLarge => ProviderTransportError::new(
                provider_id,
                ProviderTransportErrorKind::ResponseTooLarge,
            ),
        })
}

fn validate_content_length(
    provider_id: &str,
    headers: &HeaderMap,
) -> Result<(), ProviderTransportError> {
    if !headers.contains_key(CONTENT_LENGTH) {
        return Ok(());
    }
    let mut values = headers.get_all(CONTENT_LENGTH).iter();
    let value = values.next().ok_or_else(|| response_headers(provider_id))?;
    if values.next().is_some() {
        return Err(response_headers(provider_id));
    }
    let bytes = trim_optional_whitespace(value.as_bytes());
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(response_headers(provider_id));
    }
    let length = std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| response_headers(provider_id))?;
    if length > MAX_COMPRESSED_BYTES {
        return Err(ProviderTransportError::new(
            provider_id,
            ProviderTransportErrorKind::ResponseTooLarge,
        )
        .with_count(length));
    }
    Ok(())
}

fn content_encoding(
    provider_id: &str,
    headers: &HeaderMap,
) -> Result<ContentEncoding, ProviderTransportError> {
    let mut values = headers.get_all(CONTENT_ENCODING).iter();
    let Some(value) = values.next() else {
        return Ok(ContentEncoding::Identity);
    };
    if values.next().is_some() {
        return Err(response_headers(provider_id));
    }
    let bytes = trim_optional_whitespace(value.as_bytes());
    if bytes.is_empty() || !bytes.is_ascii() || bytes.contains(&b',') || bytes.contains(&b';') {
        return Err(response_headers(provider_id));
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
        Err(response_headers(provider_id))
    }
}

async fn collect_compressed(
    provider_id: &str,
    body: &mut dyn HttpBody,
    total_deadline: Instant,
) -> Result<Vec<u8>, ProviderTransportError> {
    let mut compressed = Vec::new();
    let mut idle_deadline = checked_idle_deadline(provider_id, total_deadline)?;
    loop {
        let now = Instant::now();
        if now >= total_deadline {
            return Err(ProviderTransportError::timeout(
                provider_id,
                ProviderTimeoutStage::Total,
            ));
        }
        if now >= idle_deadline {
            return Err(ProviderTransportError::timeout(
                provider_id,
                ProviderTimeoutStage::BodyIdle,
            ));
        }
        let deadline = idle_deadline.min(total_deadline);
        let chunk = strict_timeout_at(deadline, body.next_chunk())
            .await
            .map_err(|_| {
                let stage = if Instant::now() >= total_deadline {
                    ProviderTimeoutStage::Total
                } else {
                    ProviderTimeoutStage::BodyIdle
                };
                ProviderTransportError::timeout(provider_id, stage)
            })?
            .map_err(|error| match error {
                BodyError::Reqwest(error) => ProviderTransportError::reqwest(provider_id, error),
                BodyError::Timeout => {
                    let stage = if Instant::now() >= total_deadline {
                        ProviderTimeoutStage::Total
                    } else {
                        ProviderTimeoutStage::BodyIdle
                    };
                    ProviderTransportError::timeout(provider_id, stage)
                }
            })?;
        let Some(chunk) = chunk else {
            return Ok(compressed);
        };
        if chunk.is_empty() {
            tokio::task::yield_now().await;
            continue;
        }
        let next_len = compressed.len().checked_add(chunk.len()).ok_or_else(|| {
            ProviderTransportError::new(provider_id, ProviderTransportErrorKind::ResponseTooLarge)
        })?;
        if next_len > MAX_COMPRESSED_BYTES {
            return Err(ProviderTransportError::new(
                provider_id,
                ProviderTransportErrorKind::ResponseTooLarge,
            )
            .with_count(next_len));
        }
        compressed.extend_from_slice(&chunk);
        idle_deadline = checked_idle_deadline(provider_id, total_deadline)?;
    }
}

async fn decode_response(
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

async fn collect_decoded<R>(mut reader: R, compressed_len: usize) -> Result<Vec<u8>, DecodeError>
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
            .ok_or(DecodeError::TooLarge)?;
        if next_len > MAX_DECODED_BYTES || next_len > ratio_limit {
            return Err(DecodeError::TooLarge);
        }
        decoded.extend_from_slice(&buffer[..read]);
        tokio::task::yield_now().await;
    }
}

fn checked_idle_deadline(
    provider_id: &str,
    total_deadline: Instant,
) -> Result<Instant, ProviderTransportError> {
    if Instant::now() >= total_deadline {
        return Err(ProviderTransportError::timeout(
            provider_id,
            ProviderTimeoutStage::Total,
        ));
    }
    Instant::now()
        .checked_add(BODY_IDLE_TIMEOUT)
        .ok_or_else(|| ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::BodyIdle))
        .map(|deadline| deadline.min(total_deadline))
}

fn response_headers(provider_id: &str) -> ProviderTransportError {
    ProviderTransportError::new(provider_id, ProviderTransportErrorKind::ResponseHeaders)
}

fn trim_optional_whitespace(mut bytes: &[u8]) -> &[u8] {
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
    use std::{
        collections::VecDeque,
        future,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_compression::tokio::write::{BrotliEncoder, GzipEncoder, ZlibEncoder};
    use async_trait::async_trait;
    use http::HeaderValue;
    use tokio::io::AsyncWriteExt;

    use super::*;

    const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

    enum BodyStep {
        Chunk(Vec<u8>),
        End,
        Pending,
        Timeout,
    }

    struct FakeBody {
        steps: VecDeque<BodyStep>,
        polls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HttpBody for FakeBody {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            match self.steps.pop_front().unwrap_or(BodyStep::End) {
                BodyStep::Chunk(chunk) => Ok(Some(chunk)),
                BodyStep::End => Ok(None),
                BodyStep::Pending => future::pending().await,
                BodyStep::Timeout => Err(BodyError::Timeout),
            }
        }
    }

    #[tokio::test]
    async fn supported_encodings_decode_within_the_bound() {
        let document = br#"{"result":"bounded"}"#.repeat(32);
        for (encoding, compressed) in [
            ("identity", document.clone()),
            ("gzip", gzip(&document).await),
            ("br", brotli(&document).await),
            ("deflate", zlib(&document).await),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_ENCODING, HeaderValue::from_str(encoding).unwrap());
            let polls = Arc::new(AtomicUsize::new(0));
            let mut body = FakeBody {
                steps: VecDeque::from([BodyStep::Chunk(compressed), BodyStep::End]),
                polls,
            };
            let decoded = collect_response_body(
                PROVIDER_ID,
                &headers,
                &mut body,
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap();
            assert_eq!(decoded, document);
        }
    }

    #[tokio::test]
    async fn content_length_and_streaming_caps_fail_before_unbounded_collection() {
        let polls = Arc::new(AtomicUsize::new(0));
        let mut body = FakeBody {
            steps: VecDeque::from([BodyStep::Pending]),
            polls: polls.clone(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&(MAX_COMPRESSED_BYTES + 1).to_string()).unwrap(),
        );
        let error = collect_response_body(
            PROVIDER_ID,
            &headers,
            &mut body,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .expect_err("oversized content length should fail");
        assert_eq!(error.kind(), ProviderTransportErrorKind::ResponseTooLarge);
        assert_eq!(polls.load(Ordering::SeqCst), 0);

        let mut body = FakeBody {
            steps: VecDeque::from([
                BodyStep::Chunk(vec![0_u8; MAX_COMPRESSED_BYTES]),
                BodyStep::Chunk(vec![0]),
            ]),
            polls: Arc::new(AtomicUsize::new(0)),
        };
        let error = collect_response_body(
            PROVIDER_ID,
            &HeaderMap::new(),
            &mut body,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .expect_err("streamed compressed overflow should fail");
        assert_eq!(error.kind(), ProviderTransportErrorKind::ResponseTooLarge);
    }

    #[tokio::test]
    async fn invalid_encoding_headers_and_expansion_ratio_fail_closed() {
        for headers in [
            HeaderMap::from_iter([(CONTENT_ENCODING, HeaderValue::from_static("gzip, br"))]),
            HeaderMap::from_iter([(CONTENT_ENCODING, HeaderValue::from_static("compress"))]),
            {
                let mut headers = HeaderMap::new();
                headers.append(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
                headers.append(CONTENT_ENCODING, HeaderValue::from_static("br"));
                headers
            },
        ] {
            let mut body = FakeBody {
                steps: VecDeque::from([BodyStep::End]),
                polls: Arc::new(AtomicUsize::new(0)),
            };
            assert_eq!(
                collect_response_body(
                    PROVIDER_ID,
                    &headers,
                    &mut body,
                    Instant::now() + Duration::from_secs(30),
                )
                .await
                .expect_err("invalid encoding should fail")
                .kind(),
                ProviderTransportErrorKind::ResponseHeaders
            );
        }
        assert_eq!(
            collect_decoded(Cursor::new(vec![0_u8; 100]), 1)
                .await
                .expect("exact ratio boundary should pass")
                .len(),
            100
        );
        assert_eq!(
            collect_decoded(Cursor::new(vec![0_u8; 101]), 1)
                .await
                .expect_err("ratio overflow should fail"),
            DecodeError::TooLarge
        );
    }

    #[tokio::test]
    async fn exact_size_boundaries_and_empty_chunks_are_accepted() {
        let decoded = collect_decoded(
            Cursor::new(vec![0_u8; MAX_DECODED_BYTES]),
            MAX_DECODED_BYTES,
        )
        .await
        .expect("exact decoded limit should pass");
        assert_eq!(decoded.len(), MAX_DECODED_BYTES);

        let polls = Arc::new(AtomicUsize::new(0));
        let mut body = FakeBody {
            steps: VecDeque::from([
                BodyStep::Chunk(Vec::new()),
                BodyStep::Chunk(b"ok".to_vec()),
                BodyStep::End,
            ]),
            polls: polls.clone(),
        };
        let collected = collect_response_body(
            PROVIDER_ID,
            &HeaderMap::new(),
            &mut body,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .expect("empty chunks should yield without terminating the stream");
        assert_eq!(collected, b"ok");
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn pending_body_is_bounded_by_idle_timeout() {
        let mut body = FakeBody {
            steps: VecDeque::from([BodyStep::Pending]),
            polls: Arc::new(AtomicUsize::new(0)),
        };
        let task = tokio::spawn(async move {
            collect_response_body(
                PROVIDER_ID,
                &HeaderMap::new(),
                &mut body,
                Instant::now() + Duration::from_secs(30),
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(BODY_IDLE_TIMEOUT).await;
        let error = task
            .await
            .unwrap()
            .expect_err("pending body should time out");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::BodyIdle));
    }

    #[tokio::test(start_paused = true)]
    async fn total_deadline_precedes_idle_and_body_timeout_errors_are_staged() {
        let mut body = FakeBody {
            steps: VecDeque::from([BodyStep::Pending]),
            polls: Arc::new(AtomicUsize::new(0)),
        };
        let task = tokio::spawn(async move {
            collect_response_body(
                PROVIDER_ID,
                &HeaderMap::new(),
                &mut body,
                Instant::now() + Duration::from_secs(5),
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        let error = task.await.unwrap().expect_err("total deadline should win");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::Total));

        let mut body = FakeBody {
            steps: VecDeque::from([BodyStep::Timeout]),
            polls: Arc::new(AtomicUsize::new(0)),
        };
        let error = collect_response_body(
            PROVIDER_ID,
            &HeaderMap::new(),
            &mut body,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .expect_err("body timeout should be normalized");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::BodyIdle));
    }

    async fn gzip(input: &[u8]) -> Vec<u8> {
        let mut encoder = GzipEncoder::new(Vec::new());
        encoder.write_all(input).await.unwrap();
        encoder.shutdown().await.unwrap();
        encoder.into_inner()
    }

    async fn brotli(input: &[u8]) -> Vec<u8> {
        let mut encoder = BrotliEncoder::new(Vec::new());
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
