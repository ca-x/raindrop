use std::{
    collections::HashSet,
    error::Error,
    fmt,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{
        CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION, RETRY_AFTER,
    },
};
use ipnet::Ipv6Net;
use reqwest::redirect::Policy;
use rustls::crypto::CryptoProvider;
use time::OffsetDateTime;
use tokio::time::Instant;
use url::Url;

use super::{
    AddressDecision, FeedUrlError, FeedUrlPolicy, NormalizedFeedUrl, OpaqueValidator, RetryAfter,
    ValidatorSet,
    deadline::strict_timeout_at,
    decode::{DecodeError, MAX_COMPRESSED_BYTES, content_encoding, decode_document},
    resolver::{
        DnsResolveError, DnsResolver, Nat64DiscoveryError, Nat64Snapshot, Nat64Snapshots,
        SystemDnsResolver, SystemNat64PrefixDiscovery,
    },
};

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const HOP_TIMEOUT: Duration = Duration::from_secs(20);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DNS_RESULTS: usize = 16;
const MAX_REDIRECTS: usize = 5;
const MAX_SNAPSHOT_REPLAYS: usize = 2;

pub enum Nat64Mode {
    Automatic,
    Disabled,
    Static(Vec<Ipv6Net>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the process TLS crypto provider conflicts with the required ring provider")]
pub struct CryptoProviderError;

pub fn install_ring_crypto_provider() -> Result<(), CryptoProviderError> {
    let ring = rustls::crypto::ring::default_provider();
    if let Some(installed) = CryptoProvider::get_default() {
        return same_crypto_provider(installed, &ring)
            .then_some(())
            .ok_or(CryptoProviderError);
    }
    match ring.install_default() {
        Ok(()) => Ok(()),
        Err(_) => CryptoProvider::get_default()
            .is_some_and(|installed| {
                same_crypto_provider(installed, &rustls::crypto::ring::default_provider())
            })
            .then_some(())
            .ok_or(CryptoProviderError),
    }
}

fn same_crypto_provider(installed: &CryptoProvider, expected: &CryptoProvider) -> bool {
    installed.cipher_suites == expected.cipher_suites
        && installed.kx_groups.len() == expected.kx_groups.len()
        && installed
            .kx_groups
            .iter()
            .zip(&expected.kx_groups)
            .all(|(installed, expected)| std::ptr::eq(*installed, *expected))
        && std::ptr::eq(
            installed.signature_verification_algorithms.all,
            expected.signature_verification_algorithms.all,
        )
        && std::ptr::eq(
            installed.signature_verification_algorithms.mapping,
            expected.signature_verification_algorithms.mapping,
        )
        && std::ptr::eq(installed.secure_random, expected.secure_random)
        && std::ptr::eq(installed.key_provider, expected.key_provider)
}

pub struct FetchRequest {
    url: NormalizedFeedUrl,
    validators: Option<ValidatorSet>,
}

impl FetchRequest {
    #[must_use]
    pub const fn new(url: NormalizedFeedUrl, validators: Option<ValidatorSet>) -> Self {
        Self { url, validators }
    }

    #[must_use]
    pub const fn url(&self) -> &NormalizedFeedUrl {
        &self.url
    }
}

pub enum FetchOutcome {
    Document {
        url: NormalizedFeedUrl,
        document: Vec<u8>,
        content_type: Option<String>,
        etag: Option<OpaqueValidator>,
        last_modified: Option<OpaqueValidator>,
    },
    NotModified {
        url: NormalizedFeedUrl,
        etag: Option<OpaqueValidator>,
        last_modified: Option<OpaqueValidator>,
    },
}

impl FetchOutcome {
    #[must_use]
    pub const fn url(&self) -> &NormalizedFeedUrl {
        match self {
            Self::Document { url, .. } | Self::NotModified { url, .. } => url,
        }
    }

    #[must_use]
    pub fn document(&self) -> Option<&[u8]> {
        match self {
            Self::Document { document, .. } => Some(document),
            Self::NotModified { .. } => None,
        }
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        match self {
            Self::Document { content_type, .. } => content_type.as_deref(),
            Self::NotModified { .. } => None,
        }
    }

    #[must_use]
    pub const fn etag(&self) -> Option<&OpaqueValidator> {
        match self {
            Self::Document { etag, .. } | Self::NotModified { etag, .. } => etag.as_ref(),
        }
    }

    #[must_use]
    pub const fn last_modified(&self) -> Option<&OpaqueValidator> {
        match self {
            Self::Document { last_modified, .. } | Self::NotModified { last_modified, .. } => {
                last_modified.as_ref()
            }
        }
    }
}

impl fmt::Debug for FetchOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document {
                url,
                document,
                content_type,
                etag,
                last_modified,
            } => formatter
                .debug_struct("Document")
                .field("url", url)
                .field("document_bytes", &document.len())
                .field("content_type", &content_type.as_ref().map(|_| "[PRESENT]"))
                .field("etag", &etag.as_ref().map(|_| "[REDACTED]"))
                .field(
                    "last_modified",
                    &last_modified.as_ref().map(|_| "[REDACTED]"),
                )
                .finish(),
            Self::NotModified {
                url,
                etag,
                last_modified,
            } => formatter
                .debug_struct("NotModified")
                .field("url", url)
                .field("etag", &etag.as_ref().map(|_| "[PRESENT]"))
                .field(
                    "last_modified",
                    &last_modified.as_ref().map(|_| "[PRESENT]"),
                )
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchTimeoutStage {
    Dns,
    Connect,
    FirstByte,
    BodyIdle,
    Hop,
    Total,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedFetchErrorKind {
    Configuration,
    Nat64Discovery,
    Dns,
    AddressCount,
    AddressDenied,
    SnapshotReplayExhausted,
    Network,
    Timeout,
    PeerMismatch,
    Response,
    Status,
    Redirect,
    RedirectLimit,
    Encoding,
    CompressedTooLarge,
    Decode,
}

enum FetchErrorSource {
    Reqwest(reqwest::Error),
    Url(FeedUrlError),
}

pub struct FeedFetchError {
    kind: FeedFetchErrorKind,
    host: String,
    count: Option<usize>,
    status: Option<StatusCode>,
    timeout_stage: Option<FetchTimeoutStage>,
    retry_after: Option<RetryAfter>,
    source: Option<FetchErrorSource>,
}

impl FeedFetchError {
    #[must_use]
    pub const fn kind(&self) -> FeedFetchErrorKind {
        self.kind
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn count(&self) -> Option<usize> {
        self.count
    }

    #[must_use]
    pub const fn status(&self) -> Option<StatusCode> {
        self.status
    }

    #[must_use]
    pub const fn timeout_stage(&self) -> Option<FetchTimeoutStage> {
        self.timeout_stage
    }

    #[must_use]
    pub const fn retry_after(&self) -> Option<RetryAfter> {
        self.retry_after
    }

    fn new(kind: FeedFetchErrorKind, host: impl Into<String>) -> Self {
        Self {
            kind,
            host: host.into(),
            count: None,
            status: None,
            timeout_stage: None,
            retry_after: None,
            source: None,
        }
    }

    fn timeout(host: &str, stage: FetchTimeoutStage) -> Self {
        let mut error = Self::new(FeedFetchErrorKind::Timeout, host);
        error.timeout_stage = Some(stage);
        error
    }

    fn reqwest(host: &str, error: reqwest::Error) -> Self {
        let mut result = Self::new(FeedFetchErrorKind::Network, host);
        result.source = Some(FetchErrorSource::Reqwest(error.without_url()));
        result
    }
}

impl fmt::Display for FeedFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "feed fetch {:?} for host {}",
            self.kind, self.host
        )?;
        if let Some(status) = self.status {
            write!(formatter, " status {}", status.as_u16())?;
        }
        if let Some(count) = self.count {
            write!(formatter, " count {count}")?;
        }
        if let Some(stage) = self.timeout_stage {
            write!(formatter, " stage {stage:?}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FeedFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeedFetchError")
            .field("class", &self.kind)
            .field("host", &self.host)
            .field("count", &self.count)
            .finish()
    }
}

impl Error for FeedFetchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.source.as_ref() {
            Some(FetchErrorSource::Reqwest(error)) => Some(error),
            Some(FetchErrorSource::Url(error)) => Some(error),
            None => None,
        }
    }
}

#[async_trait]
pub trait FeedTransport: Send + Sync {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError>;
}

pub struct HttpFeedTransport {
    url_policy: FeedUrlPolicy,
    resolver: Arc<dyn DnsResolver>,
    snapshots: Arc<dyn SnapshotProvider>,
    executor: Arc<dyn HttpExecutor>,
}

impl HttpFeedTransport {
    pub fn new(url_policy: FeedUrlPolicy) -> Result<Self, FeedFetchError> {
        Self::with_nat64_mode(url_policy, Nat64Mode::Automatic)
    }

    pub fn with_nat64_mode(
        url_policy: FeedUrlPolicy,
        mode: Nat64Mode,
    ) -> Result<Self, FeedFetchError> {
        install_ring_crypto_provider()
            .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Configuration, "tls-provider"))?;
        let resolver = SystemDnsResolver::new()
            .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Configuration, "system-dns"))?;
        let snapshots = match mode {
            Nat64Mode::Automatic => {
                let discovery = SystemNat64PrefixDiscovery::new().map_err(|_| {
                    FeedFetchError::new(FeedFetchErrorKind::Configuration, "nat64-discovery")
                })?;
                Nat64Snapshots::automatic(Arc::new(discovery))
            }
            Nat64Mode::Disabled => Nat64Snapshots::disabled(),
            Nat64Mode::Static(prefixes) => {
                Nat64Snapshots::static_prefixes(prefixes).map_err(|_| {
                    FeedFetchError::new(FeedFetchErrorKind::Configuration, "nat64-static")
                })?
            }
        };
        Ok(Self {
            url_policy,
            resolver: Arc::new(resolver),
            snapshots: Arc::new(snapshots),
            executor: Arc::new(ReqwestExecutor::production()),
        })
    }

    #[cfg(test)]
    pub(super) fn with_parts(
        url_policy: FeedUrlPolicy,
        resolver: Arc<dyn DnsResolver>,
        snapshots: Arc<dyn SnapshotProvider>,
        executor: Arc<dyn HttpExecutor>,
    ) -> Self {
        Self {
            url_policy,
            resolver,
            snapshots,
            executor,
        }
    }
}

#[async_trait]
impl FeedTransport for HttpFeedTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        let total_deadline = Instant::now().checked_add(TOTAL_TIMEOUT).ok_or_else(|| {
            FeedFetchError::timeout(request.url.canonical_host(), FetchTimeoutStage::Total)
        })?;
        self.fetch_until(request, total_deadline).await
    }
}

impl HttpFeedTransport {
    async fn fetch_until(
        &self,
        request: FetchRequest,
        total_deadline: Instant,
    ) -> Result<FetchOutcome, FeedFetchError> {
        let mut current_url = request.url;
        let validators = request.validators;
        let mut redirects = 0_usize;

        loop {
            let host = current_url.canonical_host();
            let hop_deadline = checked_deadline(host, total_deadline, HOP_TIMEOUT)?;
            let mut snapshot = self.current_snapshot(host, total_deadline).await?;
            let dns_deadline = min_three(
                checked_deadline(host, total_deadline, DNS_TIMEOUT)?,
                hop_deadline,
                total_deadline,
            );
            let mut replays = 0_usize;
            let (mut response, approved) = loop {
                let approved = self
                    .resolve_approved(
                        &current_url,
                        &snapshot,
                        dns_deadline,
                        hop_deadline,
                        total_deadline,
                    )
                    .await?;

                let after_dns = self.current_snapshot(host, total_deadline).await?;
                if !snapshot.same_version(&after_dns) {
                    consume_replay(&mut replays, host)?;
                    snapshot = after_dns;
                    continue;
                }

                let before_executor = self.current_snapshot(host, total_deadline).await?;
                if !snapshot.same_version(&before_executor) {
                    consume_replay(&mut replays, host)?;
                    snapshot = before_executor;
                    continue;
                }

                let reusable = validators
                    .as_ref()
                    .and_then(|set| set.for_request(&current_url));
                let execute_request = HttpExecuteRequest {
                    url: current_url.complete().to_owned(),
                    host: host.to_owned(),
                    approved: approved.clone(),
                    if_none_match: reusable.and_then(|headers| headers.etag()),
                    if_modified_since: reusable.and_then(|headers| headers.last_modified()),
                    request_timeout: remaining_request_budget(host, hop_deadline, total_deadline)?,
                };

                let first_byte_deadline = min_three(
                    checked_deadline(host, total_deadline, FIRST_BYTE_TIMEOUT)?,
                    hop_deadline,
                    total_deadline,
                );
                let response =
                    strict_timeout_at(first_byte_deadline, self.executor.execute(execute_request))
                        .await
                        .map_err(|_| {
                            deadline_error(
                                host,
                                first_byte_deadline,
                                hop_deadline,
                                total_deadline,
                                FetchTimeoutStage::FirstByte,
                            )
                        })?
                        .map_err(|error| match error {
                            ExecuteError::Configuration => {
                                FeedFetchError::new(FeedFetchErrorKind::Configuration, host)
                            }
                            ExecuteError::Reqwest(error) => FeedFetchError::reqwest(host, error),
                            ExecuteError::ConnectTimeout => {
                                if Instant::now() >= total_deadline {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::Total)
                                } else if Instant::now() >= hop_deadline {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::Hop)
                                } else {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::Connect)
                                }
                            }
                            ExecuteError::FirstByteTimeout => {
                                if Instant::now() >= total_deadline {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::Total)
                                } else if Instant::now() >= hop_deadline {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::Hop)
                                } else {
                                    FeedFetchError::timeout(host, FetchTimeoutStage::FirstByte)
                                }
                            }
                        })?;
                break (response, approved);
            };

            if !response.peer.is_some_and(|peer| approved.contains(&peer)) {
                let mut error = FeedFetchError::new(FeedFetchErrorKind::PeerMismatch, host);
                error.count = Some(approved.len());
                return Err(error);
            }

            match response.status {
                StatusCode::OK => {
                    let encoding = content_encoding(&response.headers).map_err(|_error| {
                        FeedFetchError::new(FeedFetchErrorKind::Encoding, host)
                    })?;
                    let etag = single_validator(&response.headers, ETAG, host)?;
                    let last_modified = single_validator(&response.headers, LAST_MODIFIED, host)?;
                    let content_type = single_content_type(&response.headers, host)?;
                    let compressed =
                        collect_body(response.body.as_mut(), host, hop_deadline, total_deadline)
                            .await?;
                    let decode_deadline = hop_deadline.min(total_deadline);
                    let document =
                        strict_timeout_at(decode_deadline, decode_document(encoding, compressed))
                            .await
                            .map_err(|_| {
                                deadline_error(
                                    host,
                                    decode_deadline,
                                    hop_deadline,
                                    total_deadline,
                                    FetchTimeoutStage::Hop,
                                )
                            })?
                            .map_err(|_error: DecodeError| {
                                FeedFetchError::new(FeedFetchErrorKind::Decode, host)
                            })?;
                    return Ok(FetchOutcome::Document {
                        url: current_url,
                        document,
                        content_type,
                        etag,
                        last_modified,
                    });
                }
                StatusCode::NOT_MODIFIED => {
                    let etag = single_validator(&response.headers, ETAG, host)?;
                    let last_modified = single_validator(&response.headers, LAST_MODIFIED, host)?;
                    return Ok(FetchOutcome::NotModified {
                        url: current_url,
                        etag,
                        last_modified,
                    });
                }
                status if is_redirect(status) => {
                    redirects += 1;
                    if redirects > MAX_REDIRECTS {
                        let mut error =
                            FeedFetchError::new(FeedFetchErrorKind::RedirectLimit, host);
                        error.count = Some(redirects);
                        return Err(error);
                    }
                    let location = single_header(&response.headers, LOCATION)
                        .ok_or_else(|| FeedFetchError::new(FeedFetchErrorKind::Redirect, host))?;
                    let location = location
                        .to_str()
                        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Redirect, host))?;
                    let base = Url::parse(current_url.complete())
                        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Redirect, host))?;
                    let joined = base
                        .join(location)
                        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Redirect, host))?;
                    current_url = self
                        .url_policy
                        .normalize_redirect(&current_url, joined.as_str())
                        .map_err(|source| {
                            let mut error = FeedFetchError::new(FeedFetchErrorKind::Redirect, host);
                            error.source = Some(FetchErrorSource::Url(source));
                            error
                        })?;
                }
                status => {
                    let retry_after =
                        parse_retry_after(&response.headers, response.received_at, host)?;
                    let mut error = FeedFetchError::new(FeedFetchErrorKind::Status, host);
                    error.status = Some(status);
                    error.retry_after = retry_after;
                    return Err(error);
                }
            }
        }
    }

    async fn current_snapshot(
        &self,
        host: &str,
        total_deadline: Instant,
    ) -> Result<Arc<Nat64Snapshot>, FeedFetchError> {
        match self.snapshots.current(total_deadline).await {
            Ok(snapshot) => Ok(snapshot),
            Err(Nat64DiscoveryError::Deadline) if Instant::now() >= total_deadline => {
                Err(FeedFetchError::timeout(host, FetchTimeoutStage::Total))
            }
            Err(_) => Err(FeedFetchError::new(
                FeedFetchErrorKind::Nat64Discovery,
                host,
            )),
        }
    }

    async fn resolve_approved(
        &self,
        url: &NormalizedFeedUrl,
        snapshot: &Nat64Snapshot,
        dns_deadline: Instant,
        hop_deadline: Instant,
        total_deadline: Instant,
    ) -> Result<Vec<SocketAddr>, FeedFetchError> {
        let host = url.canonical_host();
        let raw = if let Ok(address) = host.parse::<IpAddr>() {
            vec![address]
        } else {
            strict_timeout_at(dns_deadline, self.resolver.resolve(host, dns_deadline))
                .await
                .map_err(|_| {
                    deadline_error(
                        host,
                        dns_deadline,
                        hop_deadline,
                        total_deadline,
                        FetchTimeoutStage::Dns,
                    )
                })?
                .map_err(|error| match error {
                    DnsResolveError::Deadline => deadline_error(
                        host,
                        dns_deadline,
                        hop_deadline,
                        total_deadline,
                        FetchTimeoutStage::Dns,
                    ),
                    DnsResolveError::Lookup => FeedFetchError::new(FeedFetchErrorKind::Dns, host),
                })?
        };
        if raw.is_empty() || raw.len() > MAX_DNS_RESULTS {
            let mut error = FeedFetchError::new(FeedFetchErrorKind::AddressCount, host);
            error.count = Some(raw.len());
            return Err(error);
        }
        if raw
            .iter()
            .any(|address| snapshot.address_policy.classify(*address) != AddressDecision::Allowed)
        {
            let mut error = FeedFetchError::new(FeedFetchErrorKind::AddressDenied, host);
            error.count = Some(raw.len());
            return Err(error);
        }

        let mut seen = HashSet::new();
        Ok(raw
            .into_iter()
            .filter(|address| seen.insert(*address))
            .map(|address| SocketAddr::new(address, url.effective_port()))
            .collect())
    }
}

fn consume_replay(replays: &mut usize, host: &str) -> Result<(), FeedFetchError> {
    if *replays >= MAX_SNAPSHOT_REPLAYS {
        let mut error = FeedFetchError::new(FeedFetchErrorKind::SnapshotReplayExhausted, host);
        error.count = Some(*replays);
        return Err(error);
    }
    *replays += 1;
    Ok(())
}

#[async_trait]
pub(super) trait SnapshotProvider: Send + Sync {
    async fn current(
        &self,
        total_deadline: Instant,
    ) -> Result<Arc<Nat64Snapshot>, Nat64DiscoveryError>;
}

#[async_trait]
impl SnapshotProvider for Nat64Snapshots {
    async fn current(
        &self,
        total_deadline: Instant,
    ) -> Result<Arc<Nat64Snapshot>, Nat64DiscoveryError> {
        self.current(total_deadline).await
    }
}

pub(super) struct HttpExecuteRequest {
    pub(super) url: String,
    pub(super) host: String,
    pub(super) approved: Vec<SocketAddr>,
    pub(super) if_none_match: Option<HeaderValue>,
    pub(super) if_modified_since: Option<HeaderValue>,
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
    #[cfg(test)]
    Other,
}

#[async_trait]
pub(super) trait HttpExecutor: Send + Sync {
    async fn execute(&self, request: HttpExecuteRequest) -> Result<HttpResponse, ExecuteError>;
}

#[async_trait]
pub(super) trait HttpBody: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError>;
}

struct ReqwestExecutor {
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl ReqwestExecutor {
    const fn production() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: BODY_IDLE_TIMEOUT,
        }
    }

    #[cfg(test)]
    const fn with_timeouts(connect_timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            connect_timeout,
            read_timeout,
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
        let mut builder = client.get(&request.url);
        if let Some(value) = request.if_none_match {
            builder = builder.header(IF_NONE_MATCH, value);
        }
        if let Some(value) = request.if_modified_since {
            builder = builder.header(IF_MODIFIED_SINCE, value);
        }
        let response = builder.send().await.map_err(|error| {
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

async fn collect_body(
    body: &mut dyn HttpBody,
    host: &str,
    hop_deadline: Instant,
    total_deadline: Instant,
) -> Result<Vec<u8>, FeedFetchError> {
    let mut compressed = Vec::new();
    let mut idle_deadline = checked_deadline(host, total_deadline, BODY_IDLE_TIMEOUT)?;
    loop {
        let now = Instant::now();
        if now >= total_deadline {
            return Err(FeedFetchError::timeout(host, FetchTimeoutStage::Total));
        }
        if now >= hop_deadline {
            return Err(FeedFetchError::timeout(host, FetchTimeoutStage::Hop));
        }
        if now >= idle_deadline {
            return Err(FeedFetchError::timeout(host, FetchTimeoutStage::BodyIdle));
        }
        let deadline = min_three(idle_deadline, hop_deadline, total_deadline);
        let chunk = strict_timeout_at(deadline, body.next_chunk())
            .await
            .map_err(|_| {
                deadline_error(
                    host,
                    deadline,
                    hop_deadline,
                    total_deadline,
                    FetchTimeoutStage::BodyIdle,
                )
            })?
            .map_err(|error| match error {
                BodyError::Reqwest(error) => FeedFetchError::reqwest(host, error),
                BodyError::Timeout => {
                    body_timeout_error(host, idle_deadline, hop_deadline, total_deadline)
                }
                #[cfg(test)]
                BodyError::Other => FeedFetchError::new(FeedFetchErrorKind::Network, host),
            })?;
        let Some(chunk) = chunk else {
            return Ok(compressed);
        };
        if chunk.is_empty() {
            tokio::task::yield_now().await;
            continue;
        }
        let next_len = compressed
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| FeedFetchError::new(FeedFetchErrorKind::CompressedTooLarge, host))?;
        if next_len > MAX_COMPRESSED_BYTES {
            let mut error = FeedFetchError::new(FeedFetchErrorKind::CompressedTooLarge, host);
            error.count = Some(next_len);
            return Err(error);
        }
        compressed.extend_from_slice(&chunk);
        idle_deadline = checked_deadline(host, total_deadline, BODY_IDLE_TIMEOUT)?;
    }
}

fn body_timeout_error(
    host: &str,
    idle_deadline: Instant,
    hop_deadline: Instant,
    total_deadline: Instant,
) -> FeedFetchError {
    let now = Instant::now();
    let stage = [
        (total_deadline, FetchTimeoutStage::Total),
        (hop_deadline, FetchTimeoutStage::Hop),
        (idle_deadline, FetchTimeoutStage::BodyIdle),
    ]
    .into_iter()
    .find_map(|(deadline, stage)| (now >= deadline).then_some(stage))
    .unwrap_or(FetchTimeoutStage::BodyIdle);
    FeedFetchError::timeout(host, stage)
}

fn single_header(headers: &HeaderMap, name: http::header::HeaderName) -> Option<&HeaderValue> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn single_validator(
    headers: &HeaderMap,
    name: http::header::HeaderName,
    host: &str,
) -> Result<Option<OpaqueValidator>, FeedFetchError> {
    if !headers.contains_key(&name) {
        return Ok(None);
    }
    let value = single_header(headers, name)
        .ok_or_else(|| FeedFetchError::new(FeedFetchErrorKind::Response, host))?;
    OpaqueValidator::from_header(value.clone())
        .map(Some)
        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Response, host))
}

fn single_content_type(headers: &HeaderMap, host: &str) -> Result<Option<String>, FeedFetchError> {
    if !headers.contains_key(CONTENT_TYPE) {
        return Ok(None);
    }
    let value = single_header(headers, CONTENT_TYPE)
        .ok_or_else(|| FeedFetchError::new(FeedFetchErrorKind::Response, host))?;
    value
        .to_str()
        .map(str::to_owned)
        .map(Some)
        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Response, host))
}

fn parse_retry_after(
    headers: &HeaderMap,
    received_at: OffsetDateTime,
    host: &str,
) -> Result<Option<RetryAfter>, FeedFetchError> {
    if !headers.contains_key(RETRY_AFTER) {
        return Ok(None);
    }
    let value = single_header(headers, RETRY_AFTER)
        .ok_or_else(|| FeedFetchError::new(FeedFetchErrorKind::Response, host))?;
    RetryAfter::parse(value, received_at)
        .map(Some)
        .map_err(|_| FeedFetchError::new(FeedFetchErrorKind::Response, host))
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn checked_deadline(
    host: &str,
    total_deadline: Instant,
    duration: Duration,
) -> Result<Instant, FeedFetchError> {
    if Instant::now() >= total_deadline {
        return Err(FeedFetchError::timeout(host, FetchTimeoutStage::Total));
    }
    Instant::now()
        .checked_add(duration)
        .ok_or_else(|| FeedFetchError::timeout(host, FetchTimeoutStage::Total))
}

fn remaining_request_budget(
    host: &str,
    hop_deadline: Instant,
    total_deadline: Instant,
) -> Result<Duration, FeedFetchError> {
    let deadline = hop_deadline.min(total_deadline);
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            deadline_error(
                host,
                deadline,
                hop_deadline,
                total_deadline,
                FetchTimeoutStage::Hop,
            )
        })
}

fn min_three(first: Instant, second: Instant, third: Instant) -> Instant {
    first.min(second).min(third)
}

fn deadline_error(
    host: &str,
    deadline: Instant,
    hop_deadline: Instant,
    total_deadline: Instant,
    default: FetchTimeoutStage,
) -> FeedFetchError {
    let stage = if deadline == total_deadline {
        FetchTimeoutStage::Total
    } else if deadline == hop_deadline {
        FetchTimeoutStage::Hop
    } else {
        default
    };
    FeedFetchError::timeout(host, stage)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error as _,
        io,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use futures_util::stream;
    use http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_ENCODING, CONTENT_TYPE, ETAG, LAST_MODIFIED, LOCATION, RETRY_AFTER},
    };
    use tokio::{sync::Notify, time::Instant};

    use super::{
        BODY_IDLE_TIMEOUT, DNS_TIMEOUT, FIRST_BYTE_TIMEOUT, FeedFetchError, FeedFetchErrorKind,
        FeedTransport, FetchOutcome, FetchRequest, FetchTimeoutStage, HOP_TIMEOUT, HttpBody,
        HttpExecuteRequest, HttpExecutor, HttpFeedTransport, HttpResponse, MAX_COMPRESSED_BYTES,
        ReqwestBody,
    };
    use crate::feeds::{
        FeedUrlPolicy, OpaqueValidator, ValidatorSet,
        resolver::{
            Nat64Discovery, Nat64DiscoveryError, Nat64DiscoveryState, Nat64PrefixDiscovery,
            Nat64Snapshot, Nat64Snapshots,
        },
        test_support::{
            BodyStep, DnsReply, FakeDnsResolver, FakeNat64Discovery, PeerSpec, ResponseSpec,
            ScriptedBody, ScriptedExecutor, ScriptedSnapshots, StalledHttpServer, event_log,
            snapshot,
        },
    };

    const PUBLIC_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
    const SECOND_PUBLIC_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
    const PRIVATE_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

    #[tokio::test]
    async fn mixed_public_and_private_dns_answers_fail_before_connect() {
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP, PRIVATE_IP,
        ])]));
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        let transport = transport(dns, stable_snapshots(), executor.clone());

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), FeedFetchErrorKind::AddressDenied);
        assert!(executor.requests().is_empty());
    }

    #[tokio::test]
    async fn dns_result_bounds_and_ip_literals_fail_closed() {
        for addresses in [Vec::new(), vec![PUBLIC_IP; 17]] {
            let executor = Arc::new(ScriptedExecutor::new(vec![]));
            let bounded_transport = transport(
                Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(addresses)])),
                stable_snapshots(),
                executor.clone(),
            );
            let error = bounded_transport
                .fetch(request("https://feed.example/rss"))
                .await
                .unwrap_err();
            assert_eq!(error.kind(), FeedFetchErrorKind::AddressCount);
            assert!(executor.requests().is_empty());
        }

        let dns = Arc::new(FakeDnsResolver::new(vec![]));
        let executor = Arc::new(ScriptedExecutor::new(vec![ResponseSpec::new(
            StatusCode::NOT_MODIFIED,
        )]));
        let literal_transport = transport(dns.clone(), stable_snapshots(), executor);
        literal_transport
            .fetch(request("https://8.8.8.8/rss"))
            .await
            .unwrap();
        assert!(dns.calls().is_empty());

        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        let private_transport = transport(
            Arc::new(FakeDnsResolver::new(vec![])),
            stable_snapshots(),
            executor.clone(),
        );
        let error = private_transport
            .fetch(request("https://10.0.0.1/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::AddressDenied);
        assert!(executor.requests().is_empty());
    }

    #[tokio::test]
    async fn resolver_is_called_once_per_hop_and_pinned_addresses_are_used() {
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP,
            PUBLIC_IP,
            SECOND_PUBLIC_IP,
        ])]));
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body =
            ScriptedBody::new(vec![BodyStep::chunk(b"feed".to_vec()), BodyStep::end()]).0;
        let executor = Arc::new(ScriptedExecutor::new(vec![response]));
        let transport = transport(dns.clone(), stable_snapshots(), executor.clone());

        let outcome = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();

        assert_eq!(outcome.document(), Some(b"feed".as_slice()));
        assert_eq!(dns.calls(), ["feed.example"]);
        let requests = executor.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].host, "feed.example");
        assert_eq!(requests[0].url, "https://feed.example/rss");
        assert!(requests[0].request_timeout <= HOP_TIMEOUT);
        assert!(!requests[0].request_timeout.is_zero());
        assert_eq!(requests[0].approved.len(), 2);
        assert!(
            requests[0]
                .approved
                .contains(&SocketAddr::new(PUBLIC_IP, 443))
        );
        assert!(
            requests[0]
                .approved
                .contains(&SocketAddr::new(SECOND_PUBLIC_IP, 443))
        );
    }

    #[tokio::test]
    async fn redirects_revalidate_private_targets_and_stop_after_five_hops() {
        let dns = Arc::new(FakeDnsResolver::new(vec![
            DnsReply::addresses(vec![PUBLIC_IP]),
            DnsReply::addresses(vec![PRIVATE_IP]),
        ]));
        let executor = Arc::new(ScriptedExecutor::new(vec![redirect(
            "https://private.example/rss",
        )]));
        let private_transport = transport(dns, stable_snapshots(), executor.clone());
        let error = private_transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::AddressDenied);
        assert_eq!(executor.requests().len(), 1);

        let dns = Arc::new(FakeDnsResolver::new(
            (0..6)
                .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                .collect(),
        ));
        let executor = Arc::new(ScriptedExecutor::new(
            (0..6)
                .map(|index| redirect(&format!("/redirect-{index}")))
                .collect(),
        ));
        let transport = transport(dns, stable_snapshots(), executor.clone());
        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::RedirectLimit);
        assert_eq!(error.count(), Some(6));
        assert_eq!(executor.requests().len(), 6);
    }

    #[tokio::test]
    async fn https_redirect_cannot_downgrade_to_http() {
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP,
        ])]));
        let executor = Arc::new(ScriptedExecutor::new(vec![redirect(
            "http://feed.example/rss",
        )]));
        let transport = transport(dns, stable_snapshots(), executor);

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::Redirect);
    }

    #[tokio::test]
    async fn redirects_require_exactly_one_location() {
        let missing = ResponseSpec::new(StatusCode::FOUND);
        let mut multiple = ResponseSpec::new(StatusCode::FOUND);
        multiple
            .headers
            .append(LOCATION, HeaderValue::from_static("/one"));
        multiple
            .headers
            .append(LOCATION, HeaderValue::from_static("/two"));
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![
                DnsReply::addresses(vec![PUBLIC_IP]),
                DnsReply::addresses(vec![PUBLIC_IP]),
            ])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![missing, multiple])),
        );
        for _ in 0..2 {
            assert_eq!(
                transport
                    .fetch(request("https://feed.example/rss"))
                    .await
                    .unwrap_err()
                    .kind(),
                FeedFetchErrorKind::Redirect
            );
        }
    }

    #[tokio::test]
    async fn validators_are_sent_only_to_the_exact_validator_url() {
        let policy = FeedUrlPolicy::new(false);
        let url = policy
            .normalize("https://feed.example/rss?edition=1")
            .unwrap();
        let validators = ValidatorSet::new(
            &url,
            Some(OpaqueValidator::from_header(HeaderValue::from_static("\"v1\"")).unwrap()),
            Some(
                OpaqueValidator::from_header(HeaderValue::from_static(
                    "Wed, 21 Oct 2015 07:28:00 GMT",
                ))
                .unwrap(),
            ),
        );
        let dns = Arc::new(FakeDnsResolver::new(vec![
            DnsReply::addresses(vec![PUBLIC_IP]),
            DnsReply::addresses(vec![PUBLIC_IP]),
        ]));
        let executor = Arc::new(ScriptedExecutor::new(vec![
            redirect("/other?edition=1"),
            ResponseSpec::new(StatusCode::NOT_MODIFIED),
        ]));
        let transport = transport(dns, stable_snapshots(), executor.clone());

        transport
            .fetch(FetchRequest::new(url, Some(validators)))
            .await
            .unwrap();

        let requests = executor.requests();
        assert!(requests[0].if_none_match.is_some());
        assert!(requests[0].if_modified_since.is_some());
        assert!(requests[1].if_none_match.is_none());
        assert!(requests[1].if_modified_since.is_none());
    }

    #[tokio::test]
    async fn not_modified_does_not_read_or_parse_a_body() {
        let (body, polls) = ScriptedBody::new(vec![BodyStep::chunk(b"must-not-read".to_vec())]);
        let mut response = ResponseSpec::new(StatusCode::NOT_MODIFIED);
        response.body = body;
        response
            .headers
            .append(CONTENT_ENCODING, HeaderValue::from_static("invalid"));
        response
            .headers
            .append(ETAG, HeaderValue::from_static("\"fresh\""));
        response.headers.append(
            LAST_MODIFIED,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP,
        ])]));
        let executor = Arc::new(ScriptedExecutor::new(vec![response]));
        let transport = transport(dns, stable_snapshots(), executor);

        let outcome = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();
        assert!(matches!(outcome, FetchOutcome::NotModified { .. }));
        assert!(outcome.etag().is_some());
        assert!(outcome.last_modified().is_some());
        assert_eq!(*polls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn status_retry_after_is_single_valid_and_never_reads_a_body() {
        let (valid_body, valid_polls) =
            ScriptedBody::new(vec![BodyStep::chunk(b"secret".to_vec())]);
        let mut valid = ResponseSpec::new(StatusCode::SERVICE_UNAVAILABLE);
        valid.body = valid_body;
        valid
            .headers
            .append(RETRY_AFTER, HeaderValue::from_static("120"));

        let (multiple_body, multiple_polls) =
            ScriptedBody::new(vec![BodyStep::chunk(b"secret".to_vec())]);
        let mut multiple = ResponseSpec::new(StatusCode::SERVICE_UNAVAILABLE);
        multiple.body = multiple_body;
        multiple
            .headers
            .append(RETRY_AFTER, HeaderValue::from_static("1"));
        multiple
            .headers
            .append(RETRY_AFTER, HeaderValue::from_static("2"));

        let (invalid_body, invalid_polls) =
            ScriptedBody::new(vec![BodyStep::chunk(b"secret".to_vec())]);
        let mut invalid = ResponseSpec::new(StatusCode::SERVICE_UNAVAILABLE);
        invalid.body = invalid_body;
        invalid
            .headers
            .append(RETRY_AFTER, HeaderValue::from_static("later"));

        let transport = transport(
            Arc::new(FakeDnsResolver::new(
                (0..3)
                    .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                    .collect(),
            )),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![valid, multiple, invalid])),
        );
        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::Status);
        assert_eq!(error.status(), Some(StatusCode::SERVICE_UNAVAILABLE));
        assert!(error.retry_after().is_some());
        for _ in 0..2 {
            assert_eq!(
                transport
                    .fetch(request("https://feed.example/rss"))
                    .await
                    .unwrap_err()
                    .kind(),
                FeedFetchErrorKind::Response
            );
        }
        assert_eq!(*valid_polls.lock().unwrap(), 0);
        assert_eq!(*multiple_polls.lock().unwrap(), 0);
        assert_eq!(*invalid_polls.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn nat64_snapshot_refreshes_by_ttl_and_blocks_fetch_when_expired() {
        let started = Instant::now();
        let discovery = Arc::new(FakeNat64Discovery::new(vec![
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: started + Duration::from_secs(1),
            }),
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: started + Duration::from_secs(2),
            }),
            Err(Nat64DiscoveryError::Lookup),
        ]));
        let snapshots = Arc::new(Nat64Snapshots::automatic(discovery.clone()));
        let dns = Arc::new(FakeDnsResolver::new(
            (0..3)
                .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                .collect(),
        ));
        let executor = Arc::new(ScriptedExecutor::new(
            (0..3)
                .map(|_| ResponseSpec::new(StatusCode::NOT_MODIFIED))
                .collect(),
        ));
        let transport = Arc::new(transport(dns.clone(), snapshots, executor));

        transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let first = tokio::spawn({
            let transport = transport.clone();
            async move { transport.fetch(request("https://feed.example/rss")).await }
        });
        let second = tokio::spawn({
            let transport = transport.clone();
            async move { transport.fetch(request("https://feed.example/rss")).await }
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(
            discovery.calls(),
            2,
            "expired refresh must be single-flight"
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), FeedFetchErrorKind::Nat64Discovery);
        assert_eq!(discovery.calls(), 3);
        assert_eq!(dns.calls().len(), 3);
    }

    #[tokio::test]
    async fn nat64_discovery_completes_before_user_controlled_dns() {
        let events = event_log();
        let discovery = Arc::new(FakeNat64Discovery::with_events(
            vec![Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: Instant::now() + Duration::from_secs(60),
            })],
            events.clone(),
        ));
        let snapshots = Arc::new(Nat64Snapshots::automatic(discovery));
        let dns = Arc::new(FakeDnsResolver::with_events(
            vec![DnsReply::addresses(vec![PUBLIC_IP])],
            events.clone(),
        ));
        let executor = Arc::new(ScriptedExecutor::new(vec![ResponseSpec::new(
            StatusCode::NOT_MODIFIED,
        )]));
        let transport = transport(dns, snapshots, executor);

        transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events[0], "discovery");
        assert_eq!(events[1], "dns");
    }

    #[tokio::test(start_paused = true)]
    async fn nat64_deadline_at_total_deadline_is_typed_as_total_timeout() {
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![])),
            Arc::new(TotalDeadlineSnapshots),
            Arc::new(ScriptedExecutor::new(vec![])),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), FeedFetchErrorKind::Timeout);
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Total));
    }

    #[tokio::test(start_paused = true)]
    async fn nat64_three_second_discovery_deadline_remains_discovery_error() {
        let started = Instant::now();
        let snapshots = Arc::new(Nat64Snapshots::automatic(Arc::new(
            ThreeSecondDeadlineDiscovery,
        )));
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![])),
            snapshots,
            Arc::new(ScriptedExecutor::new(vec![])),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), FeedFetchErrorKind::Nat64Discovery);
        assert_eq!(Instant::now() - started, DNS_TIMEOUT);
    }

    #[tokio::test]
    async fn nat64_snapshot_change_during_user_dns_discards_and_reresolves() {
        let dns = Arc::new(FakeDnsResolver::new(vec![
            DnsReply::addresses(vec![PUBLIC_IP]),
            DnsReply::addresses(vec![SECOND_PUBLIC_IP]),
        ]));
        let snapshots = Arc::new(ScriptedSnapshots::sequence(vec![snapshot(1), snapshot(2)]));
        let executor = Arc::new(ScriptedExecutor::new(vec![ResponseSpec::new(
            StatusCode::NOT_MODIFIED,
        )]));
        let transport = transport(dns.clone(), snapshots, executor.clone());

        transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();
        assert_eq!(dns.calls().len(), 2);
        assert_eq!(
            executor.requests()[0].approved,
            [SocketAddr::new(SECOND_PUBLIC_IP, 443)]
        );
    }

    #[tokio::test]
    async fn nat64_snapshot_change_before_executor_discards_and_reresolves() {
        let dns = Arc::new(FakeDnsResolver::new(vec![
            DnsReply::addresses(vec![PUBLIC_IP]),
            DnsReply::addresses(vec![SECOND_PUBLIC_IP]),
        ]));
        let snapshots = Arc::new(ScriptedSnapshots::sequence(vec![
            snapshot(1),
            snapshot(1),
            snapshot(2),
        ]));
        let executor = Arc::new(ScriptedExecutor::new(vec![ResponseSpec::new(
            StatusCode::NOT_MODIFIED,
        )]));
        let transport = transport(dns.clone(), snapshots, executor.clone());

        transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap();
        assert_eq!(dns.calls().len(), 2);
        assert_eq!(
            executor.requests()[0].approved,
            [SocketAddr::new(SECOND_PUBLIC_IP, 443)]
        );
    }

    #[tokio::test]
    async fn nat64_snapshot_replay_exhaustion_is_typed_and_bounded() {
        let dns = Arc::new(FakeDnsResolver::new(
            (0..3)
                .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                .collect(),
        ));
        let snapshots = Arc::new(ScriptedSnapshots::sequence((1..=6).map(snapshot).collect()));
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        let transport = transport(dns.clone(), snapshots, executor.clone());

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::SnapshotReplayExhausted);
        assert_eq!(error.count(), Some(2));
        assert_eq!(dns.calls().len(), 3);
        assert!(executor.requests().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_replays_share_the_original_user_dns_deadline() {
        let started = Instant::now();
        let dns = Arc::new(FakeDnsResolver::new(vec![
            DnsReply::delayed(Duration::from_secs(2), vec![PUBLIC_IP]),
            DnsReply::delayed(Duration::from_secs(2), vec![PUBLIC_IP]),
        ]));
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        let transport = transport(
            dns.clone(),
            Arc::new(ScriptedSnapshots::sequence(vec![snapshot(1), snapshot(2)])),
            executor.clone(),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Dns));
        assert_eq!(dns.calls().len(), 2);
        assert!(executor.requests().is_empty());
        assert_eq!(Instant::now() - started, Duration::from_secs(3));
    }

    #[tokio::test]
    async fn compressed_and_decoded_limits_stop_streaming() {
        let (body, polls) = ScriptedBody::new(vec![
            BodyStep::chunk(vec![0_u8; MAX_COMPRESSED_BYTES]),
            BodyStep::chunk(vec![1]),
            BodyStep::chunk(b"unreachable".to_vec()),
        ]);
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body = body;
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP,
        ])]));
        let executor = Arc::new(ScriptedExecutor::new(vec![response]));
        let transport = transport(dns, stable_snapshots(), executor);

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), FeedFetchErrorKind::CompressedTooLarge);
        assert_eq!(*polls.lock().unwrap(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn decoding_obeys_the_absolute_hop_deadline_while_in_progress() {
        let decode_start = Arc::new(Notify::new());
        let transport = HttpFeedTransport::with_parts(
            FeedUrlPolicy::new(false),
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(DecodeDeadlineExecutor {
                decode_start: decode_start.clone(),
                bytes: MAX_COMPRESSED_BYTES,
            }),
        );
        let advance = tokio::spawn(async move {
            decode_start.notified().await;
            tokio::time::advance(HOP_TIMEOUT).await;
        });

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        advance.await.unwrap();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Hop));
    }

    #[tokio::test(start_paused = true)]
    async fn decode_completion_at_exact_deadline_is_rejected() {
        let decode_start = Arc::new(Notify::new());
        let transport = HttpFeedTransport::with_parts(
            FeedUrlPolicy::new(false),
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(DecodeDeadlineExecutor {
                decode_start: decode_start.clone(),
                bytes: 1,
            }),
        );
        let advance = tokio::spawn(async move {
            decode_start.notified().await;
            tokio::time::advance(HOP_TIMEOUT).await;
        });

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        advance.await.unwrap();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Hop));
    }

    #[tokio::test(start_paused = true)]
    async fn decoding_distinguishes_the_absolute_total_deadline() {
        let decode_start = Arc::new(Notify::new());
        let transport = HttpFeedTransport::with_parts(
            FeedUrlPolicy::new(false),
            Arc::new(FakeDnsResolver::new(
                (0..3)
                    .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                    .collect(),
            )),
            stable_snapshots(),
            Arc::new(TotalDecodeDeadlineExecutor {
                calls: AtomicUsize::new(0),
                decode_start: decode_start.clone(),
            }),
        );
        let advance = tokio::spawn(async move {
            decode_start.notified().await;
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_secs(12)).await;
        });

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        advance.await.unwrap();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Total));
    }

    #[tokio::test(start_paused = true)]
    async fn timeouts_share_one_total_refresh_deadline() {
        let dns = Arc::new(FakeDnsResolver::new(
            (0..5)
                .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                .collect(),
        ));
        let executor = Arc::new(ScriptedExecutor::new(
            (0..5)
                .map(|index| {
                    let mut response = redirect(&format!("/slow-{index}"));
                    response.delay = Duration::from_secs(7);
                    response
                })
                .collect(),
        ));
        let transport = transport(dns, stable_snapshots(), executor);

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Total));
    }

    #[tokio::test(start_paused = true)]
    async fn first_byte_timeout_is_distinct_from_body_idle_timeout() {
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.delay = Duration::from_secs(11);
        let dns = Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
            PUBLIC_IP,
        ])]));
        let executor = Arc::new(ScriptedExecutor::new(vec![response]));
        let transport = transport(dns, stable_snapshots(), executor);

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::FirstByte));
    }

    #[tokio::test(start_paused = true)]
    async fn first_byte_completion_at_exact_deadline_is_rejected() {
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.delay = FIRST_BYTE_TIMEOUT;
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::FirstByte));
    }

    #[tokio::test]
    async fn connect_timeout_is_distinct_from_first_byte_timeout() {
        let transport = HttpFeedTransport::with_parts(
            FeedUrlPolicy::new(false),
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ConnectTimeoutExecutor),
        );
        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Connect));
    }

    #[tokio::test(start_paused = true)]
    async fn expired_hop_request_budget_is_not_mislabeled_as_total() {
        let error = super::remaining_request_budget(
            "feed.example",
            Instant::now(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Hop));
    }

    #[tokio::test(start_paused = true)]
    async fn body_idle_timeout_resets_only_after_a_chunk() {
        let (body, _) = ScriptedBody::new(vec![
            BodyStep::chunk(b"a".to_vec()),
            BodyStep::delayed_chunk(Duration::from_secs(9), b"b".to_vec()),
            BodyStep::delayed_end(Duration::from_secs(9)),
        ]);
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body = body;
        let successful_transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );
        assert_eq!(
            successful_transport
                .fetch(request("https://feed.example/rss"))
                .await
                .unwrap()
                .document(),
            Some(b"ab".as_slice())
        );

        let (body, _) = ScriptedBody::new(vec![
            BodyStep::delayed_chunk(Duration::from_secs(6), Vec::new()),
            BodyStep::delayed_chunk(Duration::from_secs(5), b"late".to_vec()),
        ]);
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body = body;
        let duplicate_transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );
        let error = duplicate_transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::BodyIdle));
    }

    #[tokio::test(start_paused = true)]
    async fn reqwest_body_timeout_preserves_typed_body_idle_classification() {
        let stream = stream::once(async {
            Err::<Vec<u8>, _>(io::Error::new(io::ErrorKind::TimedOut, "timed out"))
        });
        let response = http::Response::new(reqwest::Body::wrap_stream(stream)).into();
        let mut body = ReqwestBody { response };
        let now = Instant::now();

        let error = super::collect_body(
            &mut body,
            "feed.example",
            now + HOP_TIMEOUT,
            now + Duration::from_secs(30),
        )
        .await
        .unwrap_err();

        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::BodyIdle));
    }

    #[tokio::test(start_paused = true)]
    async fn body_chunk_at_exact_idle_deadline_is_rejected() {
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body = ScriptedBody::new(vec![
            BodyStep::delayed_chunk(BODY_IDLE_TIMEOUT, b"late".to_vec()),
            BodyStep::end(),
        ])
        .0;
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::BodyIdle));
    }

    #[tokio::test(start_paused = true)]
    async fn body_end_at_exact_idle_deadline_is_rejected() {
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.body = ScriptedBody::new(vec![BodyStep::delayed_end(BODY_IDLE_TIMEOUT)]).0;
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::BodyIdle));
    }

    #[tokio::test(start_paused = true)]
    async fn user_dns_completion_at_exact_deadline_is_rejected() {
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::delayed(
                DNS_TIMEOUT,
                vec![PUBLIC_IP],
            )])),
            stable_snapshots(),
            executor.clone(),
        );

        let error = transport
            .fetch(request("https://feed.example/rss"))
            .await
            .unwrap_err();
        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::Dns));
        assert!(executor.requests().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_ties_prioritize_total_then_hop_then_stage() {
        let deadline = Instant::now();
        assert_eq!(
            super::deadline_error(
                "feed.example",
                deadline,
                deadline,
                deadline,
                FetchTimeoutStage::Dns,
            )
            .timeout_stage(),
            Some(FetchTimeoutStage::Total)
        );
        assert_eq!(
            super::deadline_error(
                "feed.example",
                deadline,
                deadline,
                deadline + Duration::from_secs(1),
                FetchTimeoutStage::Dns,
            )
            .timeout_stage(),
            Some(FetchTimeoutStage::Hop)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn redirects_reset_hop_but_not_total_deadline() {
        let dns = Arc::new(FakeDnsResolver::new(
            (0..3)
                .map(|_| DnsReply::addresses(vec![PUBLIC_IP]))
                .collect(),
        ));
        let mut first = redirect("/two");
        first.delay = Duration::from_secs(9);
        let mut second = redirect("/three");
        second.delay = Duration::from_secs(9);
        let mut final_response = ResponseSpec::new(StatusCode::OK);
        final_response.delay = Duration::from_secs(9);
        final_response.body =
            ScriptedBody::new(vec![BodyStep::chunk(b"done".to_vec()), BodyStep::end()]).0;
        let transport = transport(
            dns,
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![first, second, final_response])),
        );

        assert_eq!(
            transport
                .fetch(request("https://feed.example/one"))
                .await
                .unwrap()
                .document(),
            Some(b"done".as_slice())
        );
    }

    #[tokio::test]
    async fn connected_peer_must_match_the_approved_set() {
        let mut missing = ResponseSpec::new(StatusCode::OK);
        missing.peer = PeerSpec::Missing;
        let mut mismatched = ResponseSpec::new(StatusCode::OK);
        mismatched.peer = PeerSpec::Fixed(SocketAddr::new(SECOND_PUBLIC_IP, 443));
        let executor = Arc::new(ScriptedExecutor::new(vec![missing, mismatched]));
        let transport = transport(
            Arc::new(FakeDnsResolver::new(vec![
                DnsReply::addresses(vec![PUBLIC_IP]),
                DnsReply::addresses(vec![PUBLIC_IP]),
            ])),
            stable_snapshots(),
            executor,
        );

        for _ in 0..2 {
            let error = transport
                .fetch(request("https://feed.example/rss"))
                .await
                .unwrap_err();
            assert_eq!(error.kind(), FeedFetchErrorKind::PeerMismatch);
        }
    }

    #[test]
    fn reqwest_errors_do_not_expose_url_queries_in_debug_or_display() {
        super::install_ring_crypto_provider().unwrap();
        let source = reqwest::Client::new()
            .get("https://example.com:bad/rss?token=supersecret")
            .build()
            .unwrap_err();
        let error = FeedFetchError::reqwest("example.com", source);

        for rendered in [
            format!("{error:?}"),
            error.to_string(),
            error.source().unwrap().to_string(),
        ] {
            assert!(!rendered.contains("supersecret"));
            assert!(!rendered.contains("?token="));
        }
    }

    #[test]
    fn production_client_builds_after_idempotent_ring_provider_installation() {
        super::install_ring_crypto_provider().unwrap();
        super::install_ring_crypto_provider().unwrap();
        let request = HttpExecuteRequest {
            url: "https://feed.example/rss".to_owned(),
            host: "feed.example".to_owned(),
            approved: vec![SocketAddr::new(PUBLIC_IP, 443)],
            if_none_match: None,
            if_modified_since: None,
            request_timeout: HOP_TIMEOUT,
        };

        assert!(
            super::ReqwestExecutor::production()
                .build_client(&request)
                .is_ok()
        );

        let mut modified = rustls::crypto::ring::default_provider();
        modified.cipher_suites.clear();
        assert!(!super::same_crypto_provider(
            rustls::crypto::CryptoProvider::get_default().unwrap(),
            &modified,
        ));
    }

    #[tokio::test]
    async fn reqwest_executor_read_timeout_is_typed_as_body_idle() {
        super::install_ring_crypto_provider().unwrap();
        let server = StalledHttpServer::start(
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let address = server.address();
        let executor =
            super::ReqwestExecutor::with_timeouts(Duration::from_secs(5), Duration::from_secs(1));
        let request = HttpExecuteRequest {
            url: format!("http://timeout.test:{}/", address.port()),
            host: "timeout.test".to_owned(),
            approved: vec![address],
            if_none_match: None,
            if_modified_since: None,
            request_timeout: Duration::from_secs(30),
        };
        let mut response = match executor.execute(request).await {
            Ok(response) => response,
            Err(super::ExecuteError::Configuration) => panic!("TLS provider configuration"),
            Err(super::ExecuteError::Reqwest(error)) => panic!("reqwest error: {error}"),
            Err(super::ExecuteError::ConnectTimeout) => panic!("connect timeout"),
            Err(super::ExecuteError::FirstByteTimeout) => panic!("first-byte timeout"),
        };
        let now = Instant::now();
        let error = super::collect_body(
            response.body.as_mut(),
            "timeout.test",
            now + HOP_TIMEOUT,
            now + Duration::from_secs(30),
        )
        .await
        .unwrap_err();

        assert_eq!(error.timeout_stage(), Some(FetchTimeoutStage::BodyIdle));
    }

    #[tokio::test]
    async fn response_metadata_is_single_utf8_and_redacted() {
        let mut response = ResponseSpec::new(StatusCode::OK);
        response.headers.append(
            CONTENT_TYPE,
            HeaderValue::from_static("application/rss+xml"),
        );
        response
            .headers
            .append(CONTENT_TYPE, HeaderValue::from_static("text/xml"));
        let multiple_transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );
        assert_eq!(
            multiple_transport
                .fetch(request("https://feed.example/rss"))
                .await
                .unwrap_err()
                .kind(),
            FeedFetchErrorKind::Response
        );

        let mut response = ResponseSpec::new(StatusCode::OK);
        response.headers.append(
            CONTENT_TYPE,
            HeaderValue::from_bytes(b"text/xml; x=\xff").unwrap(),
        );
        let non_utf8_transport = transport(
            Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                PUBLIC_IP,
            ])])),
            stable_snapshots(),
            Arc::new(ScriptedExecutor::new(vec![response])),
        );
        assert_eq!(
            non_utf8_transport
                .fetch(request("https://feed.example/rss"))
                .await
                .unwrap_err()
                .kind(),
            FeedFetchErrorKind::Response
        );

        for name in [ETAG, LAST_MODIFIED] {
            let mut response = ResponseSpec::new(StatusCode::OK);
            response
                .headers
                .append(name.clone(), HeaderValue::from_static("first"));
            response
                .headers
                .append(name, HeaderValue::from_static("second"));
            let metadata_transport = transport(
                Arc::new(FakeDnsResolver::new(vec![DnsReply::addresses(vec![
                    PUBLIC_IP,
                ])])),
                stable_snapshots(),
                Arc::new(ScriptedExecutor::new(vec![response])),
            );
            assert_eq!(
                metadata_transport
                    .fetch(request("https://feed.example/rss"))
                    .await
                    .unwrap_err()
                    .kind(),
                FeedFetchErrorKind::Response
            );
        }
    }

    fn transport(
        dns: Arc<FakeDnsResolver>,
        snapshots: Arc<dyn super::SnapshotProvider>,
        executor: Arc<ScriptedExecutor>,
    ) -> HttpFeedTransport {
        HttpFeedTransport::with_parts(FeedUrlPolicy::new(false), dns, snapshots, executor)
    }

    fn stable_snapshots() -> Arc<ScriptedSnapshots> {
        Arc::new(ScriptedSnapshots::stable(snapshot(1)))
    }

    fn request(raw: &str) -> FetchRequest {
        FetchRequest::new(FeedUrlPolicy::new(false).normalize(raw).unwrap(), None)
    }

    fn redirect(location: &str) -> ResponseSpec {
        let mut response = ResponseSpec::new(StatusCode::FOUND);
        response
            .headers
            .insert(LOCATION, HeaderValue::from_str(location).unwrap());
        response
    }

    struct TotalDeadlineSnapshots;

    #[async_trait]
    impl super::SnapshotProvider for TotalDeadlineSnapshots {
        async fn current(
            &self,
            total_deadline: Instant,
        ) -> Result<Arc<Nat64Snapshot>, Nat64DiscoveryError> {
            tokio::time::sleep_until(total_deadline).await;
            Err(Nat64DiscoveryError::Deadline)
        }
    }

    struct ThreeSecondDeadlineDiscovery;

    #[async_trait]
    impl Nat64PrefixDiscovery for ThreeSecondDeadlineDiscovery {
        async fn discover(&self, deadline: Instant) -> Result<Nat64Discovery, Nat64DiscoveryError> {
            tokio::time::sleep_until(deadline).await;
            Err(Nat64DiscoveryError::Deadline)
        }
    }

    struct ConnectTimeoutExecutor;

    struct DecodeDeadlineBody {
        bytes: Option<Vec<u8>>,
        decode_start: Arc<Notify>,
    }

    struct DecodeDeadlineExecutor {
        decode_start: Arc<Notify>,
        bytes: usize,
    }

    struct TotalDecodeDeadlineExecutor {
        calls: AtomicUsize,
        decode_start: Arc<Notify>,
    }

    #[async_trait]
    impl HttpExecutor for DecodeDeadlineExecutor {
        async fn execute(
            &self,
            request: HttpExecuteRequest,
        ) -> Result<HttpResponse, super::ExecuteError> {
            Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                peer: request.approved.first().copied(),
                received_at: time::OffsetDateTime::now_utc(),
                body: Box::new(DecodeDeadlineBody {
                    bytes: Some(vec![b'x'; self.bytes]),
                    decode_start: self.decode_start.clone(),
                }),
            })
        }
    }

    #[async_trait]
    impl HttpExecutor for TotalDecodeDeadlineExecutor {
        async fn execute(
            &self,
            request: HttpExecuteRequest,
        ) -> Result<HttpResponse, super::ExecuteError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < 2 {
                tokio::time::sleep(Duration::from_secs(9)).await;
                let mut headers = HeaderMap::new();
                headers.insert(LOCATION, HeaderValue::from_static("/next"));
                return Ok(HttpResponse {
                    status: StatusCode::FOUND,
                    headers,
                    peer: request.approved.first().copied(),
                    received_at: time::OffsetDateTime::now_utc(),
                    body: Box::new(ScriptedBody::empty()),
                });
            }
            Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                peer: request.approved.first().copied(),
                received_at: time::OffsetDateTime::now_utc(),
                body: Box::new(DecodeDeadlineBody {
                    bytes: Some(vec![b'x'; MAX_COMPRESSED_BYTES]),
                    decode_start: self.decode_start.clone(),
                }),
            })
        }
    }

    #[async_trait]
    impl HttpBody for DecodeDeadlineBody {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, super::BodyError> {
            if self.bytes.is_some() {
                return Ok(self.bytes.take());
            }
            self.decode_start.notify_one();
            Ok(None)
        }
    }

    #[async_trait]
    impl HttpExecutor for ConnectTimeoutExecutor {
        async fn execute(
            &self,
            _request: HttpExecuteRequest,
        ) -> Result<HttpResponse, super::ExecuteError> {
            Err(super::ExecuteError::ConnectTimeout)
        }
    }
}
