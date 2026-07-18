use std::{net::SocketAddr, time::Duration};

use async_trait::async_trait;
use http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{
        CONNECTION, CONTENT_LENGTH, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
        TRANSFER_ENCODING, UPGRADE,
    },
};
use reqwest::redirect::Policy;
use secrecy::ExposeSecret;
use time::OffsetDateTime;
use url::Url;

use crate::{content::provider::ProviderHeader, feeds::install_ring_crypto_provider};

use super::{ProviderTransportError, ProviderTransportErrorKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct HttpExecuteRequest {
    pub(super) url: Url,
    pub(super) host: String,
    pub(super) approved: Vec<SocketAddr>,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) request_timeout: Duration,
}

pub(super) struct HttpResponse {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) peer: Option<SocketAddr>,
    pub(super) received_at: OffsetDateTime,
    pub(super) body: Box<dyn HttpBody>,
}

pub(super) enum ExecuteError {
    Configuration,
    Reqwest(reqwest::Error),
    ConnectTimeout,
    FirstByteTimeout,
}

pub(super) enum BodyError {
    Reqwest(reqwest::Error),
    Timeout,
}

#[async_trait]
pub(super) trait HttpExecutor: Send + Sync {
    async fn execute(&self, request: HttpExecuteRequest) -> Result<HttpResponse, ExecuteError>;
}

#[async_trait]
pub(super) trait HttpBody: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError>;
}

pub(super) struct ReqwestExecutor {
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl ReqwestExecutor {
    pub(super) const fn production() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
        }
    }

    fn build_client(&self, request: &HttpExecuteRequest) -> Result<reqwest::Client, ExecuteError> {
        install_ring_crypto_provider().map_err(|_| ExecuteError::Configuration)?;
        reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .connect_timeout(self.connect_timeout)
            .read_timeout(self.read_timeout)
            .timeout(request.request_timeout)
            .resolve_to_addrs(&request.host, &request.approved)
            .build()
            .map_err(|error| ExecuteError::Reqwest(error.without_url()))
    }
}

#[async_trait]
impl HttpExecutor for ReqwestExecutor {
    async fn execute(&self, request: HttpExecuteRequest) -> Result<HttpResponse, ExecuteError> {
        let client = self.build_client(&request)?;
        let response = client
            .post(request.url)
            .headers(request.headers)
            .body(request.body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() && error.is_connect() {
                    ExecuteError::ConnectTimeout
                } else if error.is_timeout() {
                    ExecuteError::FirstByteTimeout
                } else {
                    ExecuteError::Reqwest(error.without_url())
                }
            })?;
        Ok(HttpResponse {
            status: response.status(),
            headers: response.headers().clone(),
            peer: response.remote_addr(),
            received_at: OffsetDateTime::now_utc(),
            body: Box::new(ReqwestBody { response }),
        })
    }
}

struct ReqwestBody {
    response: reqwest::Response,
}

#[async_trait]
impl HttpBody for ReqwestBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|error| {
                if error.is_timeout() {
                    BodyError::Timeout
                } else {
                    BodyError::Reqwest(error.without_url())
                }
            })
    }
}

pub(super) fn convert_headers(
    provider_id: &str,
    headers: &[ProviderHeader],
) -> Result<HeaderMap, ProviderTransportError> {
    let mut converted = HeaderMap::with_capacity(headers.len());
    for header in headers {
        if forbidden_header(header.name()) || converted.contains_key(header.name()) {
            return Err(ProviderTransportError::new(
                provider_id,
                ProviderTransportErrorKind::InvalidHeaders,
            ));
        }
        let mut value = match (header.public_value(), header.secret_value()) {
            (Some(value), None) => value.clone(),
            (None, Some(value)) => HeaderValue::from_str(value.expose_secret()).map_err(|_| {
                ProviderTransportError::new(provider_id, ProviderTransportErrorKind::InvalidHeaders)
            })?,
            _ => {
                return Err(ProviderTransportError::new(
                    provider_id,
                    ProviderTransportErrorKind::InvalidHeaders,
                ));
            }
        };
        if header.is_secret() {
            value.set_sensitive(true);
        }
        converted.insert(header.name().clone(), value);
    }
    Ok(converted)
}

fn forbidden_header(name: &HeaderName) -> bool {
    [
        HOST,
        CONTENT_LENGTH,
        CONNECTION,
        TRANSFER_ENCODING,
        UPGRADE,
        PROXY_AUTHORIZATION,
        PROXY_AUTHENTICATE,
        TE,
        TRAILER,
    ]
    .iter()
    .any(|forbidden| name == forbidden)
        || matches!(name.as_str(), "keep-alive" | "proxy-connection")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::provider::{EncodedProviderRequest, ProviderHeader};
    use http::header::{AUTHORIZATION, CONTENT_TYPE};
    use secrecy::SecretString;

    const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

    #[test]
    fn header_conversion_marks_secrets_and_rejects_ambiguous_headers() {
        let converted = convert_headers(
            PROVIDER_ID,
            &[
                ProviderHeader::public(CONTENT_TYPE, HeaderValue::from_static("application/json")),
                ProviderHeader::secret(AUTHORIZATION, SecretString::from("Bearer test-value")),
            ],
        )
        .unwrap();
        assert_eq!(converted[CONTENT_TYPE], "application/json");
        assert_eq!(converted[AUTHORIZATION], "Bearer test-value");
        assert!(converted[AUTHORIZATION].is_sensitive());

        let mut rejected = vec![
            vec![ProviderHeader::secret(
                AUTHORIZATION,
                SecretString::from("bad\nvalue"),
            )],
            vec![
                ProviderHeader::public(CONTENT_TYPE, HeaderValue::from_static("application/json")),
                ProviderHeader::public(CONTENT_TYPE, HeaderValue::from_static("application/json")),
            ],
        ];
        for name in [
            HOST,
            CONTENT_LENGTH,
            CONNECTION,
            TRANSFER_ENCODING,
            UPGRADE,
            PROXY_AUTHORIZATION,
            PROXY_AUTHENTICATE,
            TE,
            TRAILER,
            HeaderName::from_static("keep-alive"),
            HeaderName::from_static("proxy-connection"),
        ] {
            rejected.push(vec![ProviderHeader::public(
                name,
                HeaderValue::from_static("forbidden"),
            )]);
        }
        for headers in rejected {
            assert_eq!(
                convert_headers(PROVIDER_ID, &headers)
                    .expect_err("invalid headers should fail")
                    .kind(),
                ProviderTransportErrorKind::InvalidHeaders
            );
        }
    }

    #[test]
    fn production_client_builds_with_pinned_addresses_and_no_automatic_features() {
        let encoded =
            EncodedProviderRequest::new("/v1/responses".to_owned(), vec![], b"{}".to_vec())
                .unwrap();
        let request = HttpExecuteRequest {
            url: Url::parse("https://provider.example/v1/responses").unwrap(),
            host: "provider.example".to_owned(),
            approved: vec![SocketAddr::from(([93, 184, 216, 34], 443))],
            headers: HeaderMap::new(),
            body: encoded.body().to_vec(),
            request_timeout: Duration::from_secs(30),
        };
        let executor = ReqwestExecutor::production();
        assert_eq!(executor.connect_timeout, Duration::from_secs(5));
        assert_eq!(executor.read_timeout, Duration::from_secs(10));
        assert!(executor.build_client(&request).is_ok());
    }
}
