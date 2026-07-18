mod body;
mod dns;
mod http;
mod types;

pub use types::{
    ProviderRetryAfter, ProviderTimeoutStage, ProviderTransportError, ProviderTransportErrorKind,
    ProviderTransportResponse,
};

use std::{future::Future, sync::Arc, time::Duration};

use ::http::{HeaderMap, header::RETRY_AFTER};
use async_trait::async_trait;
use tokio::time::{Instant, timeout_at};

use crate::{
    content::provider::{EncodedProviderRequest, ProviderEndpoint},
    feeds::install_ring_crypto_provider,
};

use self::{
    body::collect_response_body,
    dns::{DnsResolver, SystemDnsResolver, resolve_approved},
    http::{ExecuteError, HttpExecuteRequest, HttpExecutor, ReqwestExecutor, convert_headers},
};

const TOTAL_TIMEOUT: Duration = Duration::from_secs(90);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(20);
const INITIALIZATION_PROVIDER_ID: &str = "transport-initialization";

pub struct HttpsProviderTransport {
    resolver: Arc<dyn DnsResolver>,
    executor: Arc<dyn HttpExecutor>,
}

impl HttpsProviderTransport {
    pub fn new() -> Result<Self, ProviderTransportError> {
        install_ring_crypto_provider().map_err(|_| {
            ProviderTransportError::new(
                INITIALIZATION_PROVIDER_ID,
                ProviderTransportErrorKind::Configuration,
            )
        })?;
        let resolver = SystemDnsResolver::new().map_err(|_| {
            ProviderTransportError::new(
                INITIALIZATION_PROVIDER_ID,
                ProviderTransportErrorKind::Configuration,
            )
        })?;
        Ok(Self {
            resolver: Arc::new(resolver),
            executor: Arc::new(ReqwestExecutor::production()),
        })
    }

    #[cfg(test)]
    fn with_parts(resolver: Arc<dyn DnsResolver>, executor: Arc<dyn HttpExecutor>) -> Self {
        Self { resolver, executor }
    }
}

#[async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn execute(
        &self,
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError>;
}

#[async_trait]
impl ProviderTransport for HttpsProviderTransport {
    async fn execute(
        &self,
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        let total_deadline = Instant::now().checked_add(TOTAL_TIMEOUT).ok_or_else(|| {
            ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::Total)
        })?;
        let url = endpoint.join_adapter_path(request.path()).map_err(|_| {
            ProviderTransportError::new(provider_id, ProviderTransportErrorKind::InvalidEndpoint)
        })?;
        let approved = resolve_approved(
            provider_id,
            endpoint,
            self.resolver.as_ref(),
            total_deadline,
        )
        .await?;
        let headers = convert_headers(provider_id, request.headers())?;
        let request_timeout = remaining_timeout(provider_id, total_deadline)?;
        let execute_request = HttpExecuteRequest {
            url,
            host: endpoint.canonical_host().to_owned(),
            approved: approved.clone(),
            headers,
            body: request.body().to_vec(),
            request_timeout,
        };
        let first_byte_deadline = Instant::now()
            .checked_add(FIRST_BYTE_TIMEOUT)
            .ok_or_else(|| {
                ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::FirstByte)
            })?
            .min(total_deadline);
        let mut response =
            strict_timeout_at(first_byte_deadline, self.executor.execute(execute_request))
                .await
                .map_err(|_| {
                    let stage = if Instant::now() >= total_deadline {
                        ProviderTimeoutStage::Total
                    } else {
                        ProviderTimeoutStage::FirstByte
                    };
                    ProviderTransportError::timeout(provider_id, stage)
                })?
                .map_err(|error| execute_error(provider_id, total_deadline, error))?;

        if !response.peer.is_some_and(|peer| approved.contains(&peer)) {
            return Err(ProviderTransportError::new(
                provider_id,
                ProviderTransportErrorKind::PeerMismatch,
            )
            .with_count(approved.len()));
        }
        let retry_after = parse_retry_after(provider_id, &response.headers, response.received_at)?;
        if response.status.is_redirection() {
            return Err(ProviderTransportError::new(
                provider_id,
                ProviderTransportErrorKind::RedirectDenied,
            ));
        }
        if !response.status.is_success() {
            return Ok(ProviderTransportResponse::new(
                response.status,
                Vec::new(),
                retry_after,
            ));
        }
        let body = collect_response_body(
            provider_id,
            &response.headers,
            response.body.as_mut(),
            total_deadline,
        )
        .await?;
        Ok(ProviderTransportResponse::new(
            response.status,
            body,
            retry_after,
        ))
    }
}

fn execute_error(
    provider_id: &str,
    total_deadline: Instant,
    error: ExecuteError,
) -> ProviderTransportError {
    match error {
        ExecuteError::Configuration => {
            ProviderTransportError::new(provider_id, ProviderTransportErrorKind::Configuration)
        }
        ExecuteError::Reqwest(error) => ProviderTransportError::reqwest(provider_id, error),
        ExecuteError::ConnectTimeout => {
            let stage = if Instant::now() >= total_deadline {
                ProviderTimeoutStage::Total
            } else {
                ProviderTimeoutStage::Connect
            };
            ProviderTransportError::timeout(provider_id, stage)
        }
        ExecuteError::FirstByteTimeout => {
            let stage = if Instant::now() >= total_deadline {
                ProviderTimeoutStage::Total
            } else {
                ProviderTimeoutStage::FirstByte
            };
            ProviderTransportError::timeout(provider_id, stage)
        }
    }
}

fn parse_retry_after(
    provider_id: &str,
    headers: &HeaderMap,
    received_at: time::OffsetDateTime,
) -> Result<Option<ProviderRetryAfter>, ProviderTransportError> {
    if !headers.contains_key(RETRY_AFTER) {
        return Ok(None);
    }
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next().ok_or_else(|| response_headers(provider_id))?;
    if values.next().is_some() {
        return Err(response_headers(provider_id));
    }
    ProviderRetryAfter::parse(value, received_at)
        .map(Some)
        .map_err(|_| response_headers(provider_id))
}

fn remaining_timeout(
    provider_id: &str,
    total_deadline: Instant,
) -> Result<Duration, ProviderTransportError> {
    total_deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::Total))
}

fn response_headers(provider_id: &str) -> ProviderTransportError {
    ProviderTransportError::new(provider_id, ProviderTransportErrorKind::ResponseHeaders)
}

struct StrictDeadlineElapsed;

async fn strict_timeout_at<F>(
    deadline: Instant,
    future: F,
) -> Result<F::Output, StrictDeadlineElapsed>
where
    F: Future,
{
    if Instant::now() >= deadline {
        return Err(StrictDeadlineElapsed);
    }
    let output = timeout_at(deadline, future)
        .await
        .map_err(|_| StrictDeadlineElapsed)?;
    if Instant::now() >= deadline {
        Err(StrictDeadlineElapsed)
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        future,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use ::http::{
        HeaderValue, StatusCode,
        header::{CONTENT_TYPE, RETRY_AFTER},
    };
    use secrecy::SecretString;

    use super::*;
    use crate::content::provider::{ProviderHeader, ProviderKind};

    use super::dns::DnsResolveError;
    use super::http::{BodyError, HttpBody, HttpResponse};

    const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";
    const PUBLIC_PEER: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)), 443);

    struct FakeResolver;

    #[async_trait]
    impl DnsResolver for FakeResolver {
        async fn resolve(&self, _host: &str) -> Result<Vec<IpAddr>, DnsResolveError> {
            Ok(vec![PUBLIC_PEER.ip()])
        }
    }

    #[derive(Clone, Copy)]
    enum PeerMode {
        Approved,
        Missing,
        Mismatched,
    }

    struct FakeExecutor {
        status: StatusCode,
        headers: HeaderMap,
        peer_mode: PeerMode,
        body_steps: Arc<Mutex<VecDeque<Option<Vec<u8>>>>>,
        body_polls: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        observed_body: Arc<Mutex<Option<Vec<u8>>>>,
        observed_url: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl HttpExecutor for FakeExecutor {
        async fn execute(&self, request: HttpExecuteRequest) -> Result<HttpResponse, ExecuteError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self
                .observed_body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(request.body);
            *self
                .observed_url
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(request.url.to_string());
            Ok(HttpResponse {
                status: self.status,
                headers: self.headers.clone(),
                peer: match self.peer_mode {
                    PeerMode::Approved => request.approved.first().copied(),
                    PeerMode::Missing => None,
                    PeerMode::Mismatched => Some(SocketAddr::from(([1, 1, 1, 1], 443))),
                },
                received_at: time::OffsetDateTime::UNIX_EPOCH,
                body: Box::new(FakeBody {
                    steps: self.body_steps.clone(),
                    polls: self.body_polls.clone(),
                }),
            })
        }
    }

    struct FakeBody {
        steps: Arc<Mutex<VecDeque<Option<Vec<u8>>>>>,
        polls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HttpBody for FakeBody {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .steps
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .unwrap_or(None))
        }
    }

    #[tokio::test]
    async fn success_posts_once_to_the_prefixed_endpoint_and_collects_the_body() {
        let executor = fake_executor(
            StatusCode::OK,
            HeaderMap::new(),
            PeerMode::Approved,
            vec![Some(br#"{"ok":true}"#.to_vec()), None],
        );
        let observed = executor.observed();
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example/prefix/"),
        )
        .unwrap();

        let response = transport
            .execute(PROVIDER_ID, &endpoint, encoded_request())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), br#"{"ok":true}"#);
        assert_eq!(observed.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            observed
                .url
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref(),
            Some("https://provider.example/prefix/v1/responses")
        );
        assert_eq!(
            observed
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref(),
            Some(br#"{"request":true}"#.as_slice())
        );
    }

    #[tokio::test]
    async fn non_success_and_redirect_responses_never_poll_the_body() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
        let executor = fake_executor(
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            PeerMode::Approved,
            vec![Some(b"credential-sentinel-body".to_vec())],
        );
        let observed = executor.observed();
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiResponses, None).unwrap();

        let response = transport
            .execute(PROVIDER_ID, &endpoint, encoded_request())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.body().is_empty());
        assert_eq!(
            response.retry_after().unwrap().at(),
            time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(5)
        );
        assert_eq!(observed.body_polls.load(Ordering::SeqCst), 0);

        let executor = fake_executor(
            StatusCode::TEMPORARY_REDIRECT,
            HeaderMap::new(),
            PeerMode::Approved,
            vec![Some(b"redirect-body".to_vec())],
        );
        let observed = executor.observed();
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        let error = transport
            .execute(PROVIDER_ID, &endpoint, encoded_request())
            .await
            .expect_err("redirect should fail");
        assert_eq!(error.kind(), ProviderTransportErrorKind::RedirectDenied);
        assert_eq!(observed.body_polls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn missing_peer_and_ambiguous_retry_after_fail_closed() {
        let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiResponses, None).unwrap();
        let executor = fake_executor(
            StatusCode::OK,
            HeaderMap::new(),
            PeerMode::Missing,
            vec![None],
        );
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        assert_eq!(
            transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
                .expect_err("missing peer should fail")
                .kind(),
            ProviderTransportErrorKind::PeerMismatch
        );

        let executor = fake_executor(
            StatusCode::OK,
            HeaderMap::new(),
            PeerMode::Mismatched,
            vec![None],
        );
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        assert_eq!(
            transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
                .expect_err("mismatched peer should fail")
                .kind(),
            ProviderTransportErrorKind::PeerMismatch
        );

        let mut headers = HeaderMap::new();
        headers.append(RETRY_AFTER, HeaderValue::from_static("1"));
        headers.append(RETRY_AFTER, HeaderValue::from_static("2"));
        let executor = fake_executor(
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            PeerMode::Approved,
            vec![None],
        );
        let transport =
            HttpsProviderTransport::with_parts(Arc::new(FakeResolver), Arc::new(executor));
        assert_eq!(
            transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
                .expect_err("multiple Retry-After values should fail")
                .kind(),
            ProviderTransportErrorKind::ResponseHeaders
        );
    }

    struct PendingExecutor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HttpExecutor for PendingExecutor {
        async fn execute(
            &self,
            _request: HttpExecuteRequest,
        ) -> Result<HttpResponse, ExecuteError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            future::pending().await
        }
    }

    struct FailingExecutor {
        failure: ExecuteFailure,
    }

    #[derive(Clone, Copy)]
    enum ExecuteFailure {
        ConnectTimeout,
        FirstByteTimeout,
        Configuration,
    }

    #[async_trait]
    impl HttpExecutor for FailingExecutor {
        async fn execute(
            &self,
            _request: HttpExecuteRequest,
        ) -> Result<HttpResponse, ExecuteError> {
            Err(match self.failure {
                ExecuteFailure::ConnectTimeout => ExecuteError::ConnectTimeout,
                ExecuteFailure::FirstByteTimeout => ExecuteError::FirstByteTimeout,
                ExecuteFailure::Configuration => ExecuteError::Configuration,
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn executor_timeouts_are_staged_and_first_byte_wait_is_bounded() {
        let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiResponses, None).unwrap();
        for (failure, stage) in [
            (
                ExecuteFailure::ConnectTimeout,
                ProviderTimeoutStage::Connect,
            ),
            (
                ExecuteFailure::FirstByteTimeout,
                ProviderTimeoutStage::FirstByte,
            ),
        ] {
            let transport = HttpsProviderTransport::with_parts(
                Arc::new(FakeResolver),
                Arc::new(FailingExecutor { failure }),
            );
            let error = transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
                .expect_err("executor timeout should fail");
            assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
            assert_eq!(error.stage(), Some(stage));
        }

        let transport = HttpsProviderTransport::with_parts(
            Arc::new(FakeResolver),
            Arc::new(FailingExecutor {
                failure: ExecuteFailure::Configuration,
            }),
        );
        assert_eq!(
            transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
                .expect_err("configuration error should fail")
                .kind(),
            ProviderTransportErrorKind::Configuration
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let transport = Arc::new(HttpsProviderTransport::with_parts(
            Arc::new(FakeResolver),
            Arc::new(PendingExecutor {
                calls: calls.clone(),
            }),
        ));
        let task = tokio::spawn(async move {
            transport
                .execute(PROVIDER_ID, &endpoint, encoded_request())
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(FIRST_BYTE_TIMEOUT).await;
        let error = task
            .await
            .unwrap()
            .expect_err("pending executor should reach the first-byte deadline");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::FirstByte));
    }

    fn encoded_request() -> EncodedProviderRequest {
        EncodedProviderRequest::new(
            "/v1/responses".to_owned(),
            vec![
                ProviderHeader::public(CONTENT_TYPE, HeaderValue::from_static("application/json")),
                ProviderHeader::secret(
                    ::http::header::AUTHORIZATION,
                    SecretString::from("Bearer credential-sentinel"),
                ),
            ],
            br#"{"request":true}"#.to_vec(),
        )
        .unwrap()
    }

    fn fake_executor(
        status: StatusCode,
        headers: HeaderMap,
        peer_mode: PeerMode,
        body_steps: Vec<Option<Vec<u8>>>,
    ) -> FakeExecutor {
        FakeExecutor {
            status,
            headers,
            peer_mode,
            body_steps: Arc::new(Mutex::new(VecDeque::from(body_steps))),
            body_polls: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            observed_body: Arc::new(Mutex::new(None)),
            observed_url: Arc::new(Mutex::new(None)),
        }
    }

    struct Observed {
        calls: Arc<AtomicUsize>,
        body_polls: Arc<AtomicUsize>,
        body: Arc<Mutex<Option<Vec<u8>>>>,
        url: Arc<Mutex<Option<String>>>,
    }

    impl FakeExecutor {
        fn observed(&self) -> Observed {
            Observed {
                calls: self.calls.clone(),
                body_polls: self.body_polls.clone(),
                body: self.observed_body.clone(),
                url: self.observed_url.clone(),
            }
        }
    }
}
