use std::{
    collections::HashSet,
    fmt,
    io::Read,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use reqwest::{Method, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::{Host, Url};
use zeroize::Zeroizing;

pub(crate) const SPEECH_TIMEOUT: Duration = Duration::from_secs(105);

const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SUBSCRIPTION_PAGES: usize = 256;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_SUBSCRIPTION_BYTES: u64 = 32 * 1024 * 1024;
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiError {
    Cancelled,
    SpeechUnsupported,
    Address,
    Transport,
    Unauthorized,
    Forbidden,
    RateLimited,
    Server,
    Response,
    LocalData,
    LocalBusy,
    Conflict,
}

impl ApiError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Cancelled => "",
            Self::SpeechUnsupported => "远端尚不支持朗读，请升级服务端。",
            Self::Address => "请输入 HTTPS 服务地址；本机调试可使用 http://127.0.0.1:端口。",
            Self::Transport => "无法连接服务器。请检查网络和服务地址后重试。",
            Self::Unauthorized => "登录信息无效或会话已过期，请重新登录。",
            Self::Forbidden => "服务器拒绝了操作，请重新登录后重试。",
            Self::RateLimited => "请求过于频繁，请稍后重试。",
            Self::Server => "服务器暂时无法处理请求，请稍后重试。",
            Self::Response => "服务器响应不兼容，请确认使用 Raindrop 0.7 或更新版本。",
            Self::LocalData => "无法打开本地数据，请检查应用数据目录的权限与可用空间。",
            Self::LocalBusy => "本地数据正在由另一个 Raindrop Desktop 实例使用。",
            Self::Conflict => "订阅已在另一端修改，请检查同步冲突后重试。",
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ApiError {}

pub(crate) fn server_url(address: &str) -> Result<Url, ApiError> {
    let mut server = Url::parse(address.trim()).map_err(|_| ApiError::Address)?;
    let loopback = match server.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain("localhost")) => true,
        _ => false,
    };
    if server.host().is_none()
        || !(server.scheme() == "https" || server.scheme() == "http" && loopback)
        || !server.username().is_empty()
        || server.password().is_some()
        || server.query().is_some()
        || server.fragment().is_some()
    {
        return Err(ApiError::Address);
    }
    if !server.path().ends_with('/') {
        server.set_path(&format!("{}/", server.path()));
    }
    Ok(server)
}

pub(crate) fn external_url(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[derive(Clone)]
pub(crate) struct Cancellation(tokio::sync::watch::Sender<bool>);

impl Default for Cancellation {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
}

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub(crate) async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[derive(Clone)]
pub(crate) struct Api {
    transport: Transport,
    server: Url,
    csrf: Zeroizing<String>,
    pub(crate) user_id: String,
}

#[derive(Clone)]
enum Transport {
    Remote {
        client: Client,
        speech: reqwest::Client,
    },
    Local(Arc<crate::local::Local>),
}

impl fmt::Debug for Api {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Api").finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    csrf_token: String,
    user: User,
}

#[derive(Deserialize)]
struct User {
    id: String,
    username: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Subscription {
    pub(crate) subscription_id: String,
    pub(crate) feed_id: String,
    pub(crate) title: String,
    pub(crate) unread_count: i64,
    pub(crate) feed_url: String,
    pub(crate) title_override: Option<String>,
    pub(crate) category_id: Option<String>,
    pub(crate) position: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubscribeResult {
    pub(crate) subscription: Subscription,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub(crate) entry_id: String,
    pub(crate) feed_id: String,
    pub(crate) feed_title: String,
    pub(crate) title: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) canonical_url: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) is_read: bool,
    pub(crate) is_starred: bool,
    #[serde(default)]
    pub(crate) content_html: String,
}

impl Entry {
    pub(crate) fn title(&self) -> String {
        self.title
            .as_deref()
            .filter(|title| !title.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| crate::i18n::tr("无标题文章").to_string())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page<T> {
    pub(crate) items: Vec<T>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Filter {
    #[default]
    All,
    Unread,
    Starred,
}

impl Filter {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "全部文章",
            Self::Unread => "未读",
            Self::Starred => "收藏",
        }
    }

    fn parameter(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Unread => "UNREAD",
            Self::Starred => "STARRED",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) filter: Filter,
    pub(crate) feed_id: Option<String>,
    pub(crate) search: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EntryPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) is_read: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) is_starred: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EntryState {
    pub(crate) entry_id: String,
    pub(crate) is_read: bool,
    pub(crate) is_starred: bool,
}

impl Api {
    pub(crate) fn local(path: std::path::PathBuf) -> Result<Self, ApiError> {
        let local = Arc::new(crate::local::Local::start(path)?);
        Self::from_local(local)
    }

    #[cfg(test)]
    pub(crate) fn isolated(path: std::path::PathBuf) -> Result<Self, ApiError> {
        Self::from_local(Arc::new(crate::local::Local::start_with_background(
            path, false,
        )?))
    }

    #[cfg(test)]
    pub(crate) fn isolated_speech(
        path: std::path::PathBuf,
        transport: Arc<dyn raindrop::content::provider::ProviderTransport>,
    ) -> Result<Self, ApiError> {
        Self::from_local(Arc::new(crate::local::Local::start_with_transport(
            path,
            false,
            Some(transport),
        )?))
    }

    fn from_local(local: Arc<crate::local::Local>) -> Result<Self, ApiError> {
        Ok(Self {
            server: Url::parse("http://localhost/").map_err(|_| ApiError::LocalData)?,
            csrf: local.csrf.clone(),
            user_id: String::new(),
            transport: Transport::Local(local),
        })
    }

    pub(crate) fn local_core(&self) -> Option<Arc<crate::local::Local>> {
        match &self.transport {
            Transport::Local(local) => Some(local.clone()),
            Transport::Remote { .. } => None,
        }
    }

    pub(crate) fn address(&self) -> String {
        self.server.to_string()
    }

    pub(crate) fn login(
        address: &str,
        login: &str,
        password: Zeroizing<String>,
    ) -> Result<(Self, String), ApiError> {
        let cookies = Arc::new(reqwest::cookie::Jar::default());
        let client = Client::builder()
            .cookie_provider(cookies.clone())
            .redirect(Policy::none())
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(8))
            .user_agent(concat!("raindrop-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ApiError::Transport)?;
        let speech = reqwest::Client::builder()
            .cookie_provider(cookies)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(SPEECH_TIMEOUT)
            .connect_timeout(Duration::from_secs(8))
            .user_agent(concat!("raindrop-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ApiError::Transport)?;
        let mut api = Self {
            transport: Transport::Remote { client, speech },
            server: server_url(address)?,
            csrf: Zeroizing::new(String::new()),
            user_id: String::new(),
        };
        #[derive(Serialize)]
        struct Credentials<'a> {
            login: &'a str,
            password: &'a str,
        }
        let body = Zeroizing::new(
            serde_json::to_string(&Credentials {
                login,
                password: &password,
            })
            .map_err(|_| ApiError::Response)?,
        );
        let session: Session = api.request(Method::POST, "api/v1/auth/login", Some(&body))?;
        if session.csrf_token.is_empty()
            || session.user.username.is_empty()
            || session.user.id.is_empty()
            || session.user.id.len() > 256
            || session.user.username.len() > 256
            || session.csrf_token.len() > 4096
        {
            return Err(ApiError::Response);
        }
        api.csrf = Zeroizing::new(session.csrf_token);
        api.user_id = session.user.id;
        Ok((api, session.user.username))
    }

    pub(crate) fn subscriptions(&self) -> Result<Vec<Subscription>, ApiError> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let started = Instant::now();
        let mut bytes_remaining = MAX_SUBSCRIPTION_BYTES;
        for _ in 0..MAX_SUBSCRIPTION_PAGES {
            let timeout = SUBSCRIPTION_TIMEOUT
                .checked_sub(started.elapsed())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(ApiError::Response)?;
            if bytes_remaining == 0 {
                return Err(ApiError::Response);
            }
            let mut url = self
                .server
                .join("api/v1/subscriptions")
                .map_err(|_| ApiError::Address)?;
            url.query_pairs_mut().append_pair("limit", "100");
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let (page, bytes): (Page<Subscription>, usize) = self.request_url_limited(
                Method::GET,
                url,
                None,
                bytes_remaining.min(MAX_RESPONSE_BYTES),
                timeout.min(Duration::from_secs(25)),
            )?;
            bytes_remaining = bytes_remaining
                .checked_sub(bytes as u64)
                .ok_or(ApiError::Response)?;
            if started.elapsed() >= SUBSCRIPTION_TIMEOUT {
                return Err(ApiError::Response);
            }
            items.extend(page.items);
            cursor = page.next_cursor;
            if items.len() > 20_000 {
                return Err(ApiError::Response);
            }
            let Some(next) = &cursor else {
                items.sort_by(|left, right| {
                    (left.position, &left.subscription_id)
                        .cmp(&(right.position, &right.subscription_id))
                });
                return Ok(items);
            };
            if next.len() > MAX_CURSOR_BYTES || !seen.insert(next.clone()) {
                return Err(ApiError::Response);
            }
        }
        Err(ApiError::Response)
    }

    pub(crate) fn subscribe(&self, address: &str) -> Result<Subscription, ApiError> {
        self.ensure_subscription(address)
            .map(|created| created.subscription)
    }

    pub(crate) fn ensure_subscription(&self, address: &str) -> Result<SubscribeResult, ApiError> {
        let body = serde_json::json!({"url": address}).to_string();
        self.request(Method::POST, "api/v1/subscriptions", Some(&body))
    }

    pub(crate) fn refresh(&self, subscription_id: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({"requestId": uuid::Uuid::new_v4().to_string()}).to_string();
        self.request::<serde_json::Value>(
            Method::POST,
            &format!("api/v1/subscriptions/{subscription_id}/refresh"),
            Some(&body),
        )
        .map(|_| ())
    }

    pub(crate) fn read<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.request(Method::GET, path, None)
    }

    pub(crate) fn write<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, ApiError> {
        self.request(method, path, Some(&body.to_string()))
    }

    pub(crate) fn entries(
        &self,
        query: &Query,
        cursor: Option<&str>,
    ) -> Result<Page<Entry>, ApiError> {
        let mut url = self
            .server
            .join("api/v1/entries")
            .map_err(|_| ApiError::Address)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("state", query.filter.parameter())
                .append_pair("limit", "50");
            if let Some(feed) = &query.feed_id {
                pairs.append_pair("feedId", feed);
            }
            if !query.search.trim().is_empty() {
                pairs.append_pair("search", query.search.trim());
            }
            if let Some(cursor) = cursor {
                pairs.append_pair("cursor", cursor);
            }
        }
        self.request_url(Method::GET, url, None)
    }

    pub(crate) fn entry(&self, id: &str) -> Result<Entry, ApiError> {
        self.request_url(Method::GET, self.entry_url(id, false)?, None)
    }

    pub(crate) fn patch(&self, id: &str, patch: EntryPatch) -> Result<EntryState, ApiError> {
        let body = serde_json::to_string(&patch).map_err(|_| ApiError::Response)?;
        self.request_url(Method::PATCH, self.entry_url(id, true)?, Some(&body))
    }

    fn entry_url(&self, id: &str, state: bool) -> Result<Url, ApiError> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ApiError::Response);
        }
        self.server
            .join(&format!(
                "api/v1/entries/{id}{}",
                if state { "/state" } else { "" }
            ))
            .map_err(|_| ApiError::Address)
    }

    pub(crate) fn logout(&self) -> Result<(), ApiError> {
        self.request::<serde_json::Value>(Method::POST, "api/v1/auth/logout", None)
            .map(|_| ())
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<T, ApiError> {
        self.request_url(
            method,
            self.server.join(path).map_err(|_| ApiError::Address)?,
            body,
        )
    }

    fn request_url<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        body: Option<&str>,
    ) -> Result<T, ApiError> {
        self.request_url_limited(
            method,
            url,
            body,
            MAX_RESPONSE_BYTES,
            Duration::from_secs(25),
        )
        .map(|(value, _)| value)
    }

    fn request_url_limited<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        body: Option<&str>,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(T, usize), ApiError> {
        let (status, bytes) = self.request_bytes(method, url, body, max_bytes, timeout)?;
        if status == 204 {
            return serde_json::from_str("null")
                .map(|value| (value, bytes.len()))
                .map_err(|_| ApiError::Response);
        }
        serde_json::from_slice(&bytes)
            .map(|value| (value, bytes.len()))
            .map_err(|_| ApiError::Response)
    }

    pub(crate) fn binary(
        &self,
        path: &str,
        body: &serde_json::Value,
        cancellation: &Cancellation,
    ) -> Result<Vec<u8>, ApiError> {
        let (_, bytes) = self.speech_bytes(path, body, cancellation)?;
        if bytes.is_empty() {
            Err(ApiError::Response)
        } else {
            Ok(bytes)
        }
    }

    pub(crate) fn speech_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        cancellation: &Cancellation,
    ) -> Result<T, ApiError> {
        let (_, bytes) = self.speech_bytes(path, body, cancellation)?;
        serde_json::from_slice(&bytes).map_err(|_| ApiError::Response)
    }

    fn speech_bytes(
        &self,
        path: &str,
        body: &serde_json::Value,
        cancellation: &Cancellation,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        if cancellation.is_cancelled() {
            return Err(ApiError::Cancelled);
        }
        let url = self.server.join(path).map_err(|_| ApiError::Address)?;
        let body = body.to_string();
        let response = match &self.transport {
            Transport::Local(local) => local.request_cancellable(
                Method::POST,
                &url[url::Position::BeforePath..url::Position::AfterQuery],
                Some(&body),
                cancellation,
                SPEECH_TIMEOUT,
            )?,
            Transport::Remote { speech, .. } => {
                static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, ApiError>> =
                    OnceLock::new();
                let runtime = RUNTIME
                    .get_or_init(|| {
                        tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(2)
                            .enable_all()
                            .build()
                            .map_err(|_| ApiError::Transport)
                    })
                    .as_ref()
                    .map_err(|error| *error)?;
                runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => Err(ApiError::Cancelled),
                        response = self.remote_speech(speech, url, body) => response,
                    }
                })?
            }
        };
        Self::check_status(response.0)?;
        Ok(response)
    }

    fn speech_request(
        &self,
        client: &reqwest::Client,
        url: Url,
        body: String,
    ) -> Result<reqwest::Request, ApiError> {
        client
            .post(url)
            .timeout(SPEECH_TIMEOUT)
            .header("accept", "application/json, audio/mpeg")
            .header("content-type", "application/json")
            .header("x-csrf-token", self.csrf.as_str())
            .body(body)
            .build()
            .map_err(|_| ApiError::Transport)
    }

    async fn remote_speech(
        &self,
        client: &reqwest::Client,
        url: Url,
        body: String,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let request = self.speech_request(client, url, body)?;
        let mut response = client
            .execute(request)
            .await
            .map_err(|_| ApiError::Transport)?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(ApiError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ApiError::Transport)? {
            if chunk.len() as u64 > MAX_RESPONSE_BYTES - bytes.len() as u64 {
                return Err(ApiError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, bytes))
    }

    fn check_status(status: u16) -> Result<(), ApiError> {
        match status {
            200..=299 => Ok(()),
            401 => Err(ApiError::Unauthorized),
            403 => Err(ApiError::Forbidden),
            409 | 412 => Err(ApiError::Conflict),
            429 => Err(ApiError::RateLimited),
            500..=599 => Err(ApiError::Server),
            _ => Err(ApiError::Response),
        }
    }

    fn request_bytes(
        &self,
        method: Method,
        url: Url,
        body: Option<&str>,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let speech_config = method == Method::GET && url.path().ends_with("/api/v1/plugins/tts");
        let (status, bytes) = match &self.transport {
            Transport::Local(local) => {
                let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
                local.request(method, path, body)?
            }
            Transport::Remote { client, .. } => {
                Self::remote_request(client, &self.csrf, method, url, body, max_bytes, timeout)?
            }
        };
        if speech_config && status == 404 {
            return Err(ApiError::SpeechUnsupported);
        }
        Self::check_status(status)?;
        if bytes.len() as u64 > max_bytes {
            return Err(ApiError::Response);
        }
        Ok((status, bytes))
    }

    fn remote_request(
        client: &Client,
        csrf: &str,
        method: Method,
        url: Url,
        body: Option<&str>,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let mut request = client
            .request(method.clone(), url)
            .timeout(timeout)
            .header("accept", "application/json, audio/mpeg");
        if method != Method::GET && !csrf.is_empty() {
            request = request.header("x-csrf-token", csrf);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_owned());
        }
        let response = request.send().map_err(|_| ApiError::Transport)?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            return Err(ApiError::Response);
        }
        let mut bytes = Vec::new();
        response
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ApiError::Transport)?;
        if bytes.len() as u64 > max_bytes {
            return Err(ApiError::Response);
        }
        Ok((status, bytes))
    }
}

pub(crate) async fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    std::thread::Builder::new()
        .name("raindrop-http".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|_| ApiError::Transport)?;
    receiver.await.map_err(|_| ApiError::Transport)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_server_addresses() {
        for address in [
            "http://example.com",
            "http://localhost.evil.test",
            "https://user:pass@example.com",
            "https://example.com/#secret",
            "https://example.com/?token=x",
            "file:///tmp/x",
            "javascript:alert(1)",
        ] {
            assert_eq!(server_url(address), Err(ApiError::Address), "{address}");
        }
        assert_eq!(
            server_url("https://example.com/reader").unwrap().as_str(),
            "https://example.com/reader/"
        );
        for address in [
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
            "http://localhost:8080",
        ] {
            assert!(server_url(address).is_ok(), "{address}");
        }
    }

    #[test]
    fn original_links_never_execute_local_schemes() {
        for address in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,test",
            "https://user:secret@example.com",
        ] {
            assert!(external_url(address).is_none(), "{address}");
        }
        assert!(external_url("https://example.com/article").is_some());
    }

    #[test]
    fn state_patch_does_not_overwrite_unrelated_state() {
        let patch = EntryPatch {
            is_read: None,
            is_starred: Some(true),
        };
        assert_eq!(
            serde_json::to_value(patch).unwrap(),
            serde_json::json!({"isStarred": true})
        );
    }
}

#[cfg(test)]
mod speech_tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    fn fixture(response: Vec<u8>) -> (Api, std::thread::JoinHandle<String>) {
        fixture_with_ready(response, None)
    }

    fn fixture_with_ready(
        response: Vec<u8>,
        ready: Option<std::sync::mpsc::Sender<()>>,
    ) -> (Api, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut captured = String::new();
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break bytes.len();
                    }
                };
                let headers = String::from_utf8(bytes.clone()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                bytes.resize(header_end + length, 0);
                stream.read_exact(&mut bytes[header_end..]).unwrap();
                if index == 0 {
                    let body =
                        r#"{"csrfToken":"speech-csrf","user":{"id":"reader","username":"reader"}}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nSet-Cookie: session=speech-session; Path=/; HttpOnly\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                } else {
                    captured = String::from_utf8(bytes).unwrap();
                    if let Some(ready) = &ready {
                        ready.send(()).unwrap();
                    }
                    if response.is_empty() {
                        let mut byte = [0];
                        assert_eq!(
                            stream.read(&mut byte).unwrap(),
                            0,
                            "cancellation must close the in-flight HTTP request"
                        );
                    } else {
                        let _ = stream.write_all(&response);
                    }
                }
            }
            captured
        });
        let api = Api::login(&address, "reader", Zeroizing::new("password".into()))
            .unwrap()
            .0;
        (api, thread)
    }

    #[test]
    fn old_remote_reports_speech_specific_upgrade_guidance() {
        let response =
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (api, server) = fixture(response);
        assert_eq!(
            api.read::<serde_json::Value>("api/v1/plugins/tts"),
            Err(ApiError::SpeechUnsupported)
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /api/v1/plugins/tts "));
    }

    #[test]
    fn remote_speech_uses_a_budget_longer_than_the_provider_deadline() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nID3".to_vec();
        let (api, server) = fixture(response);
        let Transport::Remote { speech, .. } = &api.transport else {
            panic!("expected remote client");
        };
        let request = api
            .speech_request(
                speech,
                api.server.join("api/v1/plugins/tts/test").unwrap(),
                "{}".into(),
            )
            .unwrap();
        assert!(
            request
                .timeout()
                .is_some_and(|timeout| *timeout >= Duration::from_secs(100))
        );
        assert_eq!(
            api.binary(
                "api/v1/plugins/tts/test",
                &serde_json::json!({}),
                &Cancellation::default()
            )
            .unwrap(),
            b"ID3"
        );
        server.join().unwrap();
    }

    #[test]
    fn cancelling_remote_speech_closes_the_pending_request() {
        let (ready, requested) = std::sync::mpsc::channel();
        let (api, server) = fixture_with_ready(Vec::new(), Some(ready));
        let cancellation = Cancellation::default();
        let token = cancellation.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = api.binary("api/v1/plugins/tts/test", &serde_json::json!({}), &token);
            done.send(result).unwrap();
        });
        requested.recv_timeout(Duration::from_secs(3)).unwrap();
        cancellation.cancel();
        assert_eq!(
            completed.recv_timeout(Duration::from_secs(3)).unwrap(),
            Err(ApiError::Cancelled)
        );
        worker.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn speech_config_round_trips_through_authenticated_in_process_router() {
        let directory = tempfile::tempdir().unwrap();
        let api = Api::isolated(directory.path().to_path_buf()).unwrap();
        let mut config: crate::speech::Config = api.read("api/v1/plugins/tts").unwrap();
        assert!(!config.is_enabled);
        assert_eq!(config.revision, 0);
        config.base_url = "https://api.openai.com/v1".into();
        config.model = "tts-1".into();
        config.voice = "zh-CN-YunyangNeural".into();
        config.speed = 1.0;
        let saved: crate::speech::Config = api
            .write(Method::PUT, "api/v1/plugins/tts", &config.draft("", false))
            .unwrap();
        assert_eq!(saved.voice, "zh-CN-YunyangNeural");
        assert_eq!(saved.revision, 1);
        assert!(
            !saved.is_enabled,
            "a disabled configuration can be saved and previewed explicitly"
        );
        let loaded: crate::speech::Config = api.read("api/v1/plugins/tts").unwrap();
        assert_eq!(loaded, saved);
    }

    #[test]
    fn speech_binary_preserves_mp3_and_authenticated_transport() {
        let mp3 = b"ID3\x00\xff\x80";
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", mp3.len()).into_bytes();
        response.extend(mp3);
        let (api, server) = fixture(response);
        let body = serde_json::json!({"voice":"zh-CN-YunyangNeural", "text":"preview"});
        assert_eq!(
            api.binary("api/v1/plugins/tts/test", &body, &Cancellation::default())
                .unwrap(),
            mp3
        );
        let request = server.join().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /api/v1/plugins/tts/test "));
        assert!(lower.contains("x-csrf-token: speech-csrf"));
        assert!(lower.contains("cookie: session=speech-session"));
        assert!(lower.contains("content-type: application/json"));
        assert!(request.ends_with(&body.to_string()));
        assert!(!request.lines().next().unwrap().contains("speech-csrf"));
    }

    #[test]
    fn speech_binary_rejects_empty_oversize_and_http_errors() {
        for (response, expected) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                ApiError::Response,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    MAX_RESPONSE_BYTES + 1
                )
                .into_bytes(),
                ApiError::Response,
            ),
            (
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
                ApiError::Unauthorized,
            ),
            (
                b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                ApiError::Conflict,
            ),
        ] {
            let (api, server) = fixture(response);
            assert_eq!(
                api.binary(
                    "api/v1/plugins/tts/test",
                    &serde_json::json!({}),
                    &Cancellation::default()
                ),
                Err(expected)
            );
            server.join().unwrap();
        }
    }
}
