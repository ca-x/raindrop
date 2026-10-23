use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes},
    http::{Method, StatusCode, header},
    response::Response,
};
use futures_util::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use url::Url;

use crate::api::{ApiError, UserConcurrencyLimiter};

const MAX_STREAM_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct TamboConfig {
    url: Url,
    api_key: SecretString,
}

impl std::fmt::Debug for TamboConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TamboConfig")
            .field("origin", &self.origin())
            .finish_non_exhaustive()
    }
}

impl TamboConfig {
    pub fn new(url: &str, api_key: SecretString) -> Result<Self, &'static str> {
        let mut url = Url::parse(url).map_err(|_| "expected an absolute HTTP(S) URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || api_key.expose_secret().trim().is_empty()
            || api_key.expose_secret().len() > 4096
            || header::HeaderValue::from_str(api_key.expose_secret()).is_err()
        {
            return Err(
                "expected an HTTP(S) URL without credentials, query or fragment and a valid API key",
            );
        }
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(Self { url, api_key })
    }

    pub fn origin(&self) -> String {
        self.url.origin().ascii_serialization()
    }
}

pub struct TamboClient {
    config: TamboConfig,
    client: reqwest::Client,
    concurrency: Arc<Semaphore>,
    users: UserConcurrencyLimiter,
}

impl TamboClient {
    pub fn new(config: TamboConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self {
            config,
            client,
            concurrency: Arc::new(Semaphore::new(8)),
            users: UserConcurrencyLimiter::new(2),
        })
    }

    pub fn origin(&self) -> String {
        self.config.origin()
    }

    pub(crate) async fn forward(
        &self,
        user_id: &str,
        method: Method,
        path: &str,
        query: Option<&str>,
        body: Bytes,
    ) -> Result<Response, ApiError> {
        if !allowed_route(&method, path) {
            return Err(ApiError::not_found());
        }
        let permit = self
            .concurrency
            .clone()
            .try_acquire_owned()
            .map_err(|_| busy())?;
        let user_permit = self.users.try_acquire(user_id).ok_or_else(busy)?;
        let mut url = self.config.url.join(path).map_err(|_| unavailable())?;
        let user_key = format!("raindrop:{user_id}");
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(query) = query {
                for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
                    if matches!(
                        name.as_ref(),
                        "limit" | "cursor" | "includeMessages" | "includeComponents"
                    ) && value.len() <= 1024
                    {
                        pairs.append_pair(&name, &value);
                    }
                }
            }
            pairs.append_pair("userKey", &user_key);
        }
        let mut request = self
            .client
            .request(method.clone(), url)
            .header("x-api-key", self.config.api_key.expose_secret())
            .header(header::ACCEPT, "application/json, text/event-stream");
        if matches!(method, Method::POST | Method::PATCH)
            || (method == Method::DELETE && !body.is_empty())
        {
            let bytes = if body.is_empty() {
                b"{}".as_slice()
            } else {
                &body
            };
            let document = bind_identity(path, bytes, &user_key)?;
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(&document).map_err(|_| invalid_request())?);
        } else if !body.is_empty() {
            return Err(invalid_request());
        }
        let upstream = request.send().await.map_err(|_| unavailable())?;
        if matches!(
            upstream.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            return Err(ApiError::not_found());
        }
        if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(busy());
        }
        if !upstream.status().is_success() {
            return Err(unavailable());
        }
        let content_type = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if upstream.status() != StatusCode::NO_CONTENT
            && !matches!(
                content_type.split(';').next(),
                Some("application/json" | "text/event-stream")
            )
        {
            return Err(unavailable());
        }
        let mut response = Response::builder()
            .status(upstream.status())
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-accel-buffering", "no");
        if !content_type.is_empty() {
            response = response.header(header::CONTENT_TYPE, content_type);
        }
        for name in ["x-thread-id", "x-run-id"] {
            if let Some(value) = upstream.headers().get(name)
                && value.as_bytes().len() <= 128
            {
                response = response.header(name, value);
            }
        }
        let mut bytes_read = 0_usize;
        let permits = (permit, user_permit);
        let stream = upstream.bytes_stream().map(move |chunk| {
            let _held_permits = &permits;
            let chunk = chunk.map_err(|_| std::io::Error::other("Tambo stream interrupted"))?;
            bytes_read = bytes_read.saturating_add(chunk.len());
            if bytes_read > MAX_STREAM_BYTES {
                return Err(std::io::Error::other("Tambo response exceeds limit"));
            }
            Ok(chunk)
        });
        response
            .body(Body::from_stream(stream))
            .map_err(|_| unavailable())
    }
}

fn allowed_route(method: &Method, path: &str) -> bool {
    let segments = path.split('/').collect::<Vec<_>>();
    let valid_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    match segments.as_slice() {
        ["v1", "threads"] => matches!(*method, Method::GET | Method::POST),
        ["v1", "threads", "runs"] if *method == Method::POST => true,
        ["v1", "threads", thread_id] if valid_id(thread_id) => {
            matches!(*method, Method::GET | Method::PATCH | Method::DELETE)
        }
        ["v1", "threads", thread_id, "messages"] if valid_id(thread_id) => *method == Method::GET,
        ["v1", "threads", thread_id, "messages", message_id]
            if valid_id(thread_id) && valid_id(message_id) =>
        {
            *method == Method::GET
        }
        ["v1", "threads", thread_id, "runs"] if valid_id(thread_id) => *method == Method::POST,
        ["v1", "threads", thread_id, "runs", run_id] if valid_id(thread_id) && valid_id(run_id) => {
            *method == Method::DELETE
        }
        [
            "v1",
            "threads",
            thread_id,
            "components",
            component_id,
            "state",
        ] if valid_id(thread_id) && valid_id(component_id) => *method == Method::POST,
        _ => false,
    }
}

fn bind_identity(path: &str, bytes: &[u8], user_key: &str) -> Result<Value, ApiError> {
    let mut document: Value = serde_json::from_slice(bytes).map_err(|_| invalid_request())?;
    let object = document.as_object_mut().ok_or_else(invalid_request)?;
    object.remove("contextKey");
    object.remove("userToken");
    object.remove("userKey");
    if path == "v1/threads/runs" {
        let thread = object.entry("thread").or_insert_with(|| json!({}));
        let thread = thread.as_object_mut().ok_or_else(invalid_request)?;
        thread.remove("contextKey");
        thread.remove("userToken");
        thread.insert("userKey".to_owned(), json!(user_key));
    } else {
        object.insert("userKey".to_owned(), json!(user_key));
    }
    Ok(document)
}

fn invalid_request() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "TAMBO_INVALID_REQUEST",
        "Invalid assistant request",
    )
}

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "TAMBO_UNAVAILABLE",
        "The optional assistant is unavailable",
    )
}

fn busy() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "TAMBO_BUSY",
        "The optional assistant is busy",
    )
    .with_header(header::RETRY_AFTER, header::HeaderValue::from_static("30"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_only_allows_user_scoped_v1_conversation_routes() {
        assert!(allowed_route(&Method::POST, "v1/threads/runs"));
        assert!(allowed_route(&Method::POST, "v1/threads/thread_1/runs"));
        assert!(allowed_route(
            &Method::DELETE,
            "v1/threads/thread_1/runs/run_1"
        ));
        for path in [
            "threads/project/admin",
            "oauth/token",
            "v1/threads/../projects",
            "v1/threads/%2fadmin",
            "v1/threads/id/messages/id/suggestions",
            "v1/projects",
            "v1/threads/id//runs",
        ] {
            assert!(!allowed_route(&Method::POST, path), "{path}");
        }
        assert!(!allowed_route(&Method::PUT, "v1/threads/id"));
    }

    #[test]
    fn proxy_binds_every_supported_identity_location_to_the_authenticated_user() {
        let body = br#"{"userKey":"victim","contextKey":"victim","userToken":"secret","thread":{"userKey":"victim"},"message":{"content":[]}}"#;
        let bound = bind_identity("v1/threads/runs", body, "raindrop:current").unwrap();
        assert!(bound.get("userKey").is_none());
        assert!(bound.get("contextKey").is_none());
        assert!(bound.get("userToken").is_none());
        assert_eq!(bound["thread"]["userKey"], "raindrop:current");
        let bound = bind_identity(
            "v1/threads/id/runs",
            br#"{"userKey":"victim"}"#,
            "raindrop:current",
        )
        .unwrap();
        assert_eq!(bound["userKey"], "raindrop:current");
        assert!(bind_identity("v1/threads/runs", br#"{"thread":null}"#, "current").is_err());
        assert!(bind_identity("v1/threads", b"[]", "current").is_err());
    }

    #[test]
    fn configuration_rejects_credential_bearing_urls_and_redacts_secrets() {
        for url in [
            "file:///tmp/api",
            "https://user:password@example.com",
            "https://example.com?key=secret",
            "https://example.com#fragment",
        ] {
            assert!(TamboConfig::new(url, SecretString::from("secret")).is_err());
        }
        let config = TamboConfig::new(
            "http://tambo:3001/api",
            SecretString::from("private-project-key"),
        )
        .unwrap();
        assert_eq!(config.origin(), "http://tambo:3001");
        assert_eq!(config.url.as_str(), "http://tambo:3001/api/");
        assert!(!format!("{config:?}").contains("private-project-key"));
    }
}
