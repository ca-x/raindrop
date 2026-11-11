use std::{
    collections::HashSet,
    fmt,
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Method, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::{Host, Url};
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SUBSCRIPTION_PAGES: usize = 256;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_SUBSCRIPTION_BYTES: u64 = 32 * 1024 * 1024;
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiError {
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
pub(crate) struct Api {
    transport: Transport,
    server: Url,
    csrf: Zeroizing<String>,
    pub(crate) user_id: String,
}

#[derive(Clone)]
enum Transport {
    Remote(Client),
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
            Transport::Remote(_) => None,
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
        let mut api = Self {
            transport: Transport::Remote(
                Client::builder()
                    .cookie_store(true)
                    .redirect(Policy::none())
                    .timeout(Duration::from_secs(25))
                    .connect_timeout(Duration::from_secs(8))
                    .user_agent(concat!("raindrop-desktop/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .map_err(|_| ApiError::Transport)?,
            ),
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
        let (status, bytes) = match &self.transport {
            Transport::Local(local) => {
                let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
                local.request(method, path, body)?
            }
            Transport::Remote(client) => {
                Self::remote_request(client, &self.csrf, method, url, body, max_bytes, timeout)?
            }
        };
        match status {
            204 => {
                return serde_json::from_str("null")
                    .map(|value| (value, bytes.len()))
                    .map_err(|_| ApiError::Response);
            }
            200..=299 => {}
            401 => return Err(ApiError::Unauthorized),
            403 => return Err(ApiError::Forbidden),
            409 | 412 => return Err(ApiError::Conflict),
            429 => return Err(ApiError::RateLimited),
            500..=599 => return Err(ApiError::Server),
            _ => return Err(ApiError::Response),
        }
        if bytes.len() as u64 > max_bytes {
            return Err(ApiError::Response);
        }
        serde_json::from_slice(&bytes)
            .map(|value| (value, bytes.len()))
            .map_err(|_| ApiError::Response)
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
            .header("accept", "application/json");
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
