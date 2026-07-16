#[cfg(not(debug_assertions))]
mod assets;

use axum::{
    body::Body,
    http::{HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

use crate::api::ApiError;

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; base-uri 'self'; connect-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'";
const HTML_CACHE_CONTROL: &str = "no-cache, no-store, must-revalidate";
#[cfg(not(debug_assertions))]
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
#[cfg(not(debug_assertions))]
const SHORT_ASSET_CACHE_CONTROL: &str = "public, max-age=3600";

#[cfg(debug_assertions)]
const DEVELOPMENT_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Raindrop development</title></head>
<body><main><h1>Raindrop web development</h1><p>Run <code>npm --prefix web run dev</code>, then open <a href="http://localhost:5173">http://localhost:5173</a>.</p></main></body>
</html>"#;

pub async fn serve(method: Method, uri: Uri) -> Response {
    if is_api_path(uri.path()) {
        let mut response = ApiError::not_found().into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return response;
    }
    if method != Method::GET && method != Method::HEAD {
        return response(
            StatusCode::METHOD_NOT_ALLOWED,
            "text/plain; charset=utf-8",
            "no-store",
            Body::empty(),
        );
    }

    #[cfg(debug_assertions)]
    let body = DEVELOPMENT_PAGE.as_bytes().to_vec();

    #[cfg(not(debug_assertions))]
    if is_asset_path(uri.path()) {
        return embedded_asset(method, uri.path());
    }

    #[cfg(not(debug_assertions))]
    let Some(body) = assets::get("index.html").map(std::borrow::Cow::into_owned) else {
        return not_found();
    };

    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    };
    response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        HTML_CACHE_CONTROL,
        body,
    )
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

#[cfg(not(debug_assertions))]
fn is_asset_path(path: &str) -> bool {
    path.starts_with("/assets/") || path.starts_with("/brand/") || path == "/favicon.ico"
}

fn response(
    status: StatusCode,
    content_type: &str,
    cache_control: &'static str,
    body: Body,
) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).expect("content type must be a valid header value"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

#[cfg(not(debug_assertions))]
fn embedded_asset(method: Method, path: &str) -> Response {
    let Some(key) = safe_asset_key(path) else {
        return not_found();
    };
    let Some(body) = assets::get(key).map(std::borrow::Cow::into_owned) else {
        return not_found();
    };
    let content_type = content_type(key);
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    };
    response(
        StatusCode::OK,
        &content_type,
        if is_content_hashed_asset(key) {
            IMMUTABLE_CACHE_CONTROL
        } else {
            SHORT_ASSET_CACHE_CONTROL
        },
        body,
    )
}

#[cfg(not(debug_assertions))]
fn is_content_hashed_asset(key: &str) -> bool {
    let Some(file_name) = key.strip_prefix("assets/") else {
        return false;
    };
    let Some((stem, _extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    stem.rsplit_once('-').is_some_and(|(_name, hash)| {
        hash.len() >= 8
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

#[cfg(not(debug_assertions))]
fn safe_asset_key(path: &str) -> Option<&str> {
    let key = path.strip_prefix('/')?;
    if key.is_empty()
        || key.contains('\\')
        || key
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(key)
}

#[cfg(not(debug_assertions))]
fn content_type(path: &str) -> String {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let essence = mime.essence_str();
    if mime.type_() == mime_guess::mime::TEXT
        || matches!(
            essence,
            "application/javascript" | "application/json" | "image/svg+xml"
        )
    {
        format!("{essence}; charset=utf-8")
    } else {
        essence.to_owned()
    }
}

#[cfg(not(debug_assertions))]
fn not_found() -> Response {
    response(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        "no-store",
        Body::from("Not found"),
    )
}
