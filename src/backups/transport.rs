use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{PayloadChecksumKind, SignableBody, SignableRequest, SigningSettings, sign},
    sign::v4,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hickory_resolver::TokioResolver;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use quick_xml::{Reader, events::Event};
use reqwest::redirect::Policy;
use secrecy::ExposeSecret;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

use crate::feeds::{AddressDecision, AddressPolicy, install_ring_crypto_provider};

use super::{
    BackupError, BackupErrorKind, BackupPublicConfig, BackupSecretConfig, ExecutionTarget,
    RetentionPolicy, S3PublicConfig, S3SecretConfig, WebDavPublicConfig, WebDavSecretConfig,
};

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LISTED_OBJECTS: usize = 10_000;

#[async_trait]
pub trait BackupTransport: Send + Sync {
    async fn test(&self, target: &ExecutionTarget) -> Result<(), BackupError>;
    async fn upload(&self, target: &ExecutionTarget, body: &[u8]) -> Result<(), BackupError>;
}

pub struct ProductionBackupTransport {
    resolver: Arc<TokioResolver>,
}

impl ProductionBackupTransport {
    pub fn new() -> Result<Self, BackupError> {
        install_ring_crypto_provider()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
        let resolver = TokioResolver::builder_tokio()
            .and_then(|builder| builder.build())
            .map_err(|_| BackupError::new(BackupErrorKind::TargetUnreachable))?;
        Ok(Self {
            resolver: Arc::new(resolver),
        })
    }

    async fn execute(
        &self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<HttpResponse, BackupError> {
        let host = url
            .host_str()
            .ok_or_else(|| BackupError::new(BackupErrorKind::TargetProtocol))?
            .to_owned();
        let approved = self.resolve_approved(&url).await?;
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(REQUEST_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .resolve_to_addrs(&host, &approved)
            .build()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
        let response = client
            .request(method, url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                let _ = error.without_url();
                BackupError::new(BackupErrorKind::TargetUnreachable)
            })?;
        if response
            .remote_addr()
            .is_some_and(|peer| !approved.iter().any(|allowed| allowed.ip() == peer.ip()))
        {
            return Err(BackupError::new(BackupErrorKind::TargetUnreachable));
        }
        let status = response.status();
        let response_headers = response.headers().clone();
        let mut bytes = Vec::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| BackupError::new(BackupErrorKind::TargetUnreachable))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(BackupError::new(BackupErrorKind::TargetProtocol));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers: response_headers,
            body: bytes,
        })
    }

    async fn resolve_approved(&self, url: &Url) -> Result<Vec<SocketAddr>, BackupError> {
        let host = url
            .host_str()
            .ok_or_else(|| BackupError::new(BackupErrorKind::TargetProtocol))?;
        let addresses: Vec<IpAddr> = if let Ok(address) = host.parse() {
            vec![address]
        } else {
            tokio::time::timeout(DNS_TIMEOUT, self.resolver.lookup_ip(host))
                .await
                .map_err(|_| BackupError::new(BackupErrorKind::TargetUnreachable))?
                .map_err(|_| BackupError::new(BackupErrorKind::TargetUnreachable))?
                .iter()
                .collect()
        };
        if addresses.is_empty() || addresses.len() > 16 {
            return Err(BackupError::new(BackupErrorKind::TargetUnreachable));
        }
        let policy = AddressPolicy::public_only();
        if addresses
            .iter()
            .any(|address| policy.classify(*address) != AddressDecision::Allowed)
        {
            return Err(BackupError::new(BackupErrorKind::TargetUnreachable));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| BackupError::new(BackupErrorKind::TargetProtocol))?;
        let mut seen = HashSet::new();
        Ok(addresses
            .into_iter()
            .filter(|address| seen.insert(*address))
            .map(|address| SocketAddr::new(address, port))
            .collect())
    }

    async fn s3_request(
        &self,
        method: Method,
        url: Url,
        region: &str,
        secret: &S3SecretConfig,
        mut headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<HttpResponse, BackupError> {
        let credentials = Credentials::new(
            secret.access_key_id.expose_secret(),
            secret.secret_access_key.expose_secret(),
            secret
                .session_token
                .as_ref()
                .map(|value| value.expose_secret().to_owned()),
            None,
            "raindrop-backup-target",
        );
        let identity = credentials.into();
        let mut settings = SigningSettings::default();
        settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(region)
            .name("s3")
            .time(SystemTime::now())
            .settings(settings)
            .build()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
            .into();
        let signable_headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| {
                value
                    .to_str()
                    .map(|value| (name.as_str(), value))
                    .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))
            })
            .collect::<Result<_, _>>()?;
        let signable = SignableRequest::new(
            method.as_str(),
            url.as_str(),
            signable_headers.into_iter(),
            SignableBody::Bytes(&body),
        )
        .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
        let instructions = sign(signable, &params)
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
            .into_parts()
            .0;
        for (name, value) in instructions.headers() {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
            let mut value = HeaderValue::from_str(value)
                .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
            if name == header::AUTHORIZATION || name.as_str() == "x-amz-security-token" {
                value.set_sensitive(true);
            }
            headers.insert(name, value);
        }
        self.execute(method, url, headers, body).await
    }

    async fn s3_list(
        &self,
        config: &S3PublicConfig,
        secret: &S3SecretConfig,
        owned_prefix: &str,
    ) -> Result<Vec<RemoteObject>, BackupError> {
        let mut continuation: Option<String> = None;
        let mut objects = Vec::new();
        loop {
            let mut url = s3_url(config, None)?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("list-type", "2");
                query.append_pair("prefix", owned_prefix);
                query.append_pair("max-keys", "1000");
                if let Some(token) = &continuation {
                    query.append_pair("continuation-token", token);
                }
            }
            let response = self
                .s3_request(
                    Method::GET,
                    url,
                    &config.region,
                    secret,
                    HeaderMap::new(),
                    Vec::new(),
                )
                .await?;
            require_success(response.status)?;
            let page = parse_s3_listing(&response.body)?;
            objects.extend(page.objects);
            if objects.len() > MAX_LISTED_OBJECTS {
                return Err(BackupError::new(BackupErrorKind::TargetProtocol));
            }
            if !page.truncated {
                break;
            }
            continuation = page.next_token;
            if continuation.is_none() {
                return Err(BackupError::new(BackupErrorKind::TargetProtocol));
            }
        }
        Ok(objects)
    }

    async fn s3_delete(
        &self,
        config: &S3PublicConfig,
        secret: &S3SecretConfig,
        key: &str,
    ) -> Result<(), BackupError> {
        let response = self
            .s3_request(
                Method::DELETE,
                s3_url(config, Some(key))?,
                &config.region,
                secret,
                HeaderMap::new(),
                Vec::new(),
            )
            .await?;
        require_success(response.status)
    }

    async fn webdav_request(
        &self,
        method: Method,
        url: Url,
        secret: &WebDavSecretConfig,
        mut headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<HttpResponse, BackupError> {
        let basic = STANDARD.encode(format!(
            "{}:{}",
            secret.username.expose_secret(),
            secret.password.expose_secret()
        ));
        let mut authorization = HeaderValue::from_str(&format!("Basic {basic}"))
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
        authorization.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, authorization);
        self.execute(method, url, headers, body).await
    }

    async fn ensure_webdav_collections(
        &self,
        config: &WebDavPublicConfig,
        secret: &WebDavSecretConfig,
        object_key: &str,
    ) -> Result<(), BackupError> {
        let parent = object_key
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let mut segments = Vec::new();
        for segment in parent.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
            let url = append_segments(&config.endpoint, &segments)?;
            let response = self
                .webdav_request(
                    Method::from_bytes(b"MKCOL")
                        .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?,
                    url,
                    secret,
                    HeaderMap::new(),
                    Vec::new(),
                )
                .await?;
            if !matches!(response.status.as_u16(), 201 | 204 | 405) {
                require_success(response.status)?;
            }
        }
        Ok(())
    }

    async fn webdav_list(
        &self,
        config: &WebDavPublicConfig,
        secret: &WebDavSecretConfig,
        owned_prefix: &str,
    ) -> Result<Vec<RemoteObject>, BackupError> {
        let url = append_segments(
            &config.endpoint,
            &owned_prefix
                .split('/')
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>(),
        )?;
        let mut headers = HeaderMap::new();
        headers.insert("depth", HeaderValue::from_static("1"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/xml"),
        );
        let response = self
            .webdav_request(
                Method::from_bytes(b"PROPFIND")
                    .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?,
                url,
                secret,
                headers,
                br#"<?xml version="1.0"?><propfind xmlns="DAV:"><prop><getlastmodified/></prop></propfind>"#.to_vec(),
            )
            .await?;
        if response.status != StatusCode::MULTI_STATUS {
            require_success(response.status)?;
        }
        parse_webdav_listing(&response.body, owned_prefix)
    }

    async fn webdav_delete(
        &self,
        config: &WebDavPublicConfig,
        secret: &WebDavSecretConfig,
        key: &str,
    ) -> Result<(), BackupError> {
        let url = append_segments(
            &config.endpoint,
            &key.split('/')
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>(),
        )?;
        let response = self
            .webdav_request(Method::DELETE, url, secret, HeaderMap::new(), Vec::new())
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            require_success(response.status)
        }
    }
}

#[async_trait]
impl BackupTransport for ProductionBackupTransport {
    async fn test(&self, target: &ExecutionTarget) -> Result<(), BackupError> {
        match (&target.config, &target.secret) {
            (BackupPublicConfig::S3(config), BackupSecretConfig::S3(secret)) => {
                let owned_prefix = target_owned_prefix(&target.object_key, config.prefix.as_str());
                let _ = self.s3_list(config, secret, &owned_prefix).await?;
                Ok(())
            }
            (BackupPublicConfig::Webdav(config), BackupSecretConfig::Webdav(secret)) => {
                let probe = if target.object_key.is_empty() {
                    format!("{}/raindrop", config.prefix.trim_matches('/'))
                } else {
                    target.object_key.clone()
                };
                self.ensure_webdav_collections(config, secret, &format!("{probe}/probe.opml"))
                    .await
            }
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }

    async fn upload(&self, target: &ExecutionTarget, body: &[u8]) -> Result<(), BackupError> {
        match (&target.config, &target.secret) {
            (BackupPublicConfig::S3(config), BackupSecretConfig::S3(secret)) => {
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/xml; charset=utf-8"),
                );
                let response = self
                    .s3_request(
                        Method::PUT,
                        s3_url(config, Some(&target.object_key))?,
                        &config.region,
                        secret,
                        headers,
                        body.to_vec(),
                    )
                    .await?;
                require_success(response.status)?;
                if target.retention != RetentionPolicy::default() {
                    let prefix = owned_parent(&target.object_key)?;
                    match self.s3_list(config, secret, prefix).await {
                        Ok(objects) => {
                            for object in retention_deletions(objects, target.retention) {
                                if let Err(error) =
                                    self.s3_delete(config, secret, &object.key).await
                                {
                                    tracing::warn!(
                                        code = error.public_code(),
                                        "S3 backup retention deletion failed"
                                    );
                                }
                            }
                        }
                        Err(error) => tracing::warn!(
                            code = error.public_code(),
                            "S3 backup retention listing failed"
                        ),
                    }
                }
                Ok(())
            }
            (BackupPublicConfig::Webdav(config), BackupSecretConfig::Webdav(secret)) => {
                self.ensure_webdav_collections(config, secret, &target.object_key)
                    .await?;
                let url = append_segments(
                    &config.endpoint,
                    &target
                        .object_key
                        .split('/')
                        .filter(|segment| !segment.is_empty())
                        .collect::<Vec<_>>(),
                )?;
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/xml; charset=utf-8"),
                );
                let response = self
                    .webdav_request(Method::PUT, url, secret, headers, body.to_vec())
                    .await?;
                require_success(response.status)?;
                if target.retention != RetentionPolicy::default() {
                    let prefix = owned_parent(&target.object_key)?;
                    match self.webdav_list(config, secret, prefix).await {
                        Ok(objects) => {
                            for object in retention_deletions(objects, target.retention) {
                                if let Err(error) =
                                    self.webdav_delete(config, secret, &object.key).await
                                {
                                    tracing::warn!(
                                        code = error.public_code(),
                                        "WebDAV backup retention deletion failed"
                                    );
                                }
                            }
                        }
                        Err(error) => tracing::warn!(
                            code = error.public_code(),
                            "WebDAV backup retention listing failed"
                        ),
                    }
                }
                Ok(())
            }
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }
}

struct HttpResponse {
    status: StatusCode,
    #[allow(dead_code)]
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteObject {
    key: String,
    modified_at: OffsetDateTime,
}

struct S3ListPage {
    objects: Vec<RemoteObject>,
    truncated: bool,
    next_token: Option<String>,
}

fn s3_url(config: &S3PublicConfig, key: Option<&str>) -> Result<Url, BackupError> {
    let mut url = Url::parse(&config.endpoint)
        .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
    if !config.path_style {
        let host = url
            .host_str()
            .ok_or_else(|| BackupError::new(BackupErrorKind::TargetProtocol))?;
        url.set_host(Some(&format!("{}.{}", config.bucket, host)))
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
    }
    let existing: Vec<String> = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
        segments.clear();
        for segment in existing {
            segments.push(&segment);
        }
        if config.path_style {
            segments.push(&config.bucket);
        }
        if let Some(key) = key {
            for segment in key.split('/').filter(|segment| !segment.is_empty()) {
                segments.push(segment);
            }
        }
    }
    Ok(url)
}

fn append_segments(base: &str, additions: &[&str]) -> Result<Url, BackupError> {
    let mut url =
        Url::parse(base).map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
    let existing: Vec<String> = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?;
    segments.clear();
    for segment in existing {
        segments.push(&segment);
    }
    for segment in additions {
        segments.push(segment);
    }
    drop(segments);
    Ok(url)
}

fn require_success(status: StatusCode) -> Result<(), BackupError> {
    if status.is_success() {
        Ok(())
    } else if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        Err(BackupError::new(BackupErrorKind::TargetAuthentication))
    } else if status.is_server_error() {
        Err(BackupError::new(BackupErrorKind::TargetUnreachable))
    } else {
        Err(BackupError::new(BackupErrorKind::TargetProtocol))
    }
}

fn owned_parent(object_key: &str) -> Result<&str, BackupError> {
    let (parent, file) = object_key
        .rsplit_once('/')
        .ok_or_else(|| BackupError::new(BackupErrorKind::CorruptData))?;
    if !recognized_filename(file)
        || !parent.contains("/raindrop/") && !parent.starts_with("raindrop/")
    {
        return Err(BackupError::new(BackupErrorKind::CorruptData));
    }
    Ok(parent)
}

fn target_owned_prefix(object_key: &str, configured_prefix: &str) -> String {
    if object_key.is_empty() {
        if configured_prefix.is_empty() {
            "raindrop/".to_owned()
        } else {
            format!("{configured_prefix}/raindrop/")
        }
    } else {
        owned_parent(object_key)
            .map_or_else(|_| "raindrop/".to_owned(), |value| format!("{value}/"))
    }
}

fn recognized_filename(file: &str) -> bool {
    file.starts_with("raindrop-subscriptions-")
        && file.ends_with(".opml")
        && file.len() <= 128
        && file
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
}

fn retention_deletions(
    mut objects: Vec<RemoteObject>,
    policy: RetentionPolicy,
) -> Vec<RemoteObject> {
    objects.retain(|object| {
        object
            .key
            .rsplit_once('/')
            .is_some_and(|(_, file)| recognized_filename(file))
    });
    objects.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| right.key.cmp(&left.key))
    });
    let cutoff = policy
        .retain_days
        .map(|days| OffsetDateTime::now_utc() - time::Duration::days(i64::from(days)));
    objects
        .into_iter()
        .enumerate()
        .filter_map(|(index, object)| {
            let over_count = policy
                .retain_count
                .is_some_and(|count| index >= usize::from(count));
            let over_age = cutoff.is_some_and(|cutoff| object.modified_at < cutoff);
            (over_count || over_age).then_some(object)
        })
        .collect()
}

fn parse_s3_listing(body: &[u8]) -> Result<S3ListPage, BackupError> {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().enable_all_checks(true);
    let mut current = String::new();
    let mut key: Option<String> = None;
    let mut modified: Option<OffsetDateTime> = None;
    let mut objects = Vec::new();
    let mut truncated = false;
    let mut next_token = None;
    let mut events = 0_usize;
    loop {
        events += 1;
        if events > 100_000 {
            return Err(BackupError::new(BackupErrorKind::TargetProtocol));
        }
        match reader
            .read_event()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
        {
            Event::Start(element) => {
                current = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                if current == "Contents" {
                    key = None;
                    modified = None;
                }
            }
            Event::Text(text) => {
                let value = text
                    .decode()
                    .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
                    .into_owned();
                match current.as_str() {
                    "Key" => key = Some(value),
                    "LastModified" => {
                        modified = OffsetDateTime::parse(&value, &Rfc3339).ok();
                    }
                    "IsTruncated" => truncated = value == "true",
                    "NextContinuationToken" => next_token = Some(value),
                    _ => {}
                }
            }
            Event::End(element) => {
                if element.local_name().as_ref() == b"Contents"
                    && let (Some(key), Some(modified_at)) = (key.take(), modified.take())
                {
                    objects.push(RemoteObject { key, modified_at });
                }
                current.clear();
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(BackupError::new(BackupErrorKind::TargetProtocol));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(S3ListPage {
        objects,
        truncated,
        next_token,
    })
}

fn parse_webdav_listing(body: &[u8], owned_prefix: &str) -> Result<Vec<RemoteObject>, BackupError> {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().enable_all_checks(true);
    let mut current = String::new();
    let mut href: Option<String> = None;
    let mut modified: Option<OffsetDateTime> = None;
    let mut objects = Vec::new();
    let mut events = 0_usize;
    loop {
        events += 1;
        if events > 100_000 {
            return Err(BackupError::new(BackupErrorKind::TargetProtocol));
        }
        match reader
            .read_event()
            .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
        {
            Event::Start(element) => {
                current = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                if current == "response" {
                    href = None;
                    modified = None;
                }
            }
            Event::Text(text) => {
                let value = text
                    .decode()
                    .map_err(|_| BackupError::new(BackupErrorKind::TargetProtocol))?
                    .into_owned();
                match current.as_str() {
                    "href" => href = Some(value),
                    "getlastmodified" => {
                        modified = httpdate::parse_http_date(&value)
                            .ok()
                            .map(OffsetDateTime::from);
                    }
                    _ => {}
                }
            }
            Event::End(element) => {
                if element.local_name().as_ref() == b"response"
                    && let (Some(href), Some(modified_at)) = (href.take(), modified.take())
                    && let Some(file) = href.trim_end_matches('/').rsplit('/').next()
                    && recognized_filename(file)
                {
                    objects.push(RemoteObject {
                        key: format!("{}/{}", owned_prefix.trim_end_matches('/'), file),
                        modified_at,
                    });
                }
                current.clear();
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(BackupError::new(BackupErrorKind::TargetProtocol));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_urls_support_path_and_virtual_host_styles() {
        let mut config = S3PublicConfig {
            endpoint: "https://objects.example/base/".to_owned(),
            region: "us-east-1".to_owned(),
            bucket: "reader-backups".to_owned(),
            prefix: String::new(),
            path_style: true,
        };
        assert_eq!(
            s3_url(&config, Some("raindrop/a.opml")).unwrap().as_str(),
            "https://objects.example/base/reader-backups/raindrop/a.opml"
        );
        config.path_style = false;
        assert_eq!(
            s3_url(&config, Some("raindrop/a.opml")).unwrap().as_str(),
            "https://reader-backups.objects.example/base/raindrop/a.opml"
        );
    }

    #[test]
    fn retention_deletes_union_of_count_and_age_without_unknown_objects() {
        let now = OffsetDateTime::now_utc();
        let objects = vec![
            RemoteObject {
                key: "owned/raindrop-subscriptions-20260722T100000Z-00000000-0000-4000-8000-000000000001.opml".to_owned(),
                modified_at: now,
            },
            RemoteObject {
                key: "owned/raindrop-subscriptions-20260720T100000Z-00000000-0000-4000-8000-000000000002.opml".to_owned(),
                modified_at: now - time::Duration::days(2),
            },
            RemoteObject {
                key: "owned/keep-me.txt".to_owned(),
                modified_at: now - time::Duration::days(30),
            },
        ];
        let deletions = retention_deletions(
            objects,
            RetentionPolicy {
                retain_count: Some(1),
                retain_days: Some(7),
            },
        );
        assert_eq!(deletions.len(), 1);
        assert!(deletions[0].key.contains("20260720"));
    }

    #[test]
    fn bounded_s3_listing_parses_objects_and_pagination() {
        let page = parse_s3_listing(br#"<?xml version="1.0"?><ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>next</NextContinuationToken><Contents><Key>owned/raindrop-subscriptions-20260722T100000Z-00000000-0000-4000-8000-000000000001.opml</Key><LastModified>2026-07-22T10:00:00Z</LastModified></Contents></ListBucketResult>"#).unwrap();
        assert!(page.truncated);
        assert_eq!(page.next_token.as_deref(), Some("next"));
        assert_eq!(page.objects.len(), 1);
    }
}
