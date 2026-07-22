#[cfg(raindrop_embedded_web)]
mod assets;

use axum::{
    body::{Body, Bytes},
    http::{HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

use crate::api::ApiError;

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; base-uri 'self'; connect-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'";
const HTML_CACHE_CONTROL: &str = "no-cache, no-store, must-revalidate";
#[cfg(raindrop_embedded_web)]
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
#[cfg(raindrop_embedded_web)]
const SHORT_ASSET_CACHE_CONTROL: &str = "public, max-age=3600";
#[cfg(raindrop_embedded_web)]
const VITE_HASH_LENGTH: usize = 8;

#[cfg(not(raindrop_embedded_web))]
const DEVELOPMENT_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Raindrop development</title></head>
<body><main><h1>Raindrop web development</h1><p>Run <code>npm --prefix web run dev</code>, then open <a href="http://localhost:5173">http://localhost:5173</a>.</p></main></body>
</html>"#;

pub async fn serve(method: Method, uri: Uri) -> Response {
    if !is_safe_request_path(uri.path()) {
        return not_found();
    }
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

    #[cfg(not(raindrop_embedded_web))]
    let body = Bytes::from_static(DEVELOPMENT_PAGE.as_bytes());

    #[cfg(raindrop_embedded_web)]
    if is_asset_path(uri.path()) {
        return embedded_asset(method, uri.path());
    }

    #[cfg(raindrop_embedded_web)]
    let Some(body) = assets::get("index.html").map(embedded_bytes) else {
        return not_found();
    };

    representation_response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        HTML_CACHE_CONTROL,
        &method,
        body,
    )
}

fn is_safe_request_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.first() != Some(&b'/') {
        return false;
    }

    let mut segment = Vec::new();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'/' => {
                if is_dot_segment(&segment) {
                    return false;
                }
                segment.clear();
                index += 1;
            }
            b'\\' => return false,
            b'%' => {
                let Some(decoded) = decode_percent_byte(bytes, index) else {
                    return false;
                };
                if matches!(decoded, b'/' | b'\\') || decoded.is_ascii_control() {
                    return false;
                }
                segment.push(decoded);
                index += 3;
            }
            byte => {
                segment.push(byte);
                index += 1;
            }
        }
    }

    !is_dot_segment(&segment)
}

fn decode_percent_byte(bytes: &[u8], index: usize) -> Option<u8> {
    let high = hex_value(*bytes.get(index + 1)?)?;
    let low = hex_value(*bytes.get(index + 2)?)?;
    Some((high << 4) | low)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_dot_segment(segment: &[u8]) -> bool {
    segment == b"." || segment == b".."
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

#[cfg(raindrop_embedded_web)]
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

fn representation_response(
    status: StatusCode,
    content_type: &str,
    cache_control: &'static str,
    method: &Method,
    representation: Bytes,
) -> Response {
    let length = representation.len();
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(representation)
    };
    let mut response = response(status, content_type, cache_control, body);
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string())
            .expect("content length must be a valid header value"),
    );
    response
}

#[cfg(raindrop_embedded_web)]
fn embedded_asset(method: Method, path: &str) -> Response {
    let Some(key) = safe_asset_key(path) else {
        return not_found();
    };
    let Some(body) = assets::get(key).map(embedded_bytes) else {
        return not_found();
    };
    let content_type = content_type(key);
    representation_response(
        StatusCode::OK,
        &content_type,
        if is_content_hashed_asset(key) {
            IMMUTABLE_CACHE_CONTROL
        } else {
            SHORT_ASSET_CACHE_CONTROL
        },
        &method,
        body,
    )
}

#[cfg(raindrop_embedded_web)]
fn embedded_bytes(data: std::borrow::Cow<'static, [u8]>) -> Bytes {
    match data {
        std::borrow::Cow::Borrowed(bytes) => Bytes::from_static(bytes),
        std::borrow::Cow::Owned(bytes) => Bytes::from(bytes),
    }
}

#[cfg(raindrop_embedded_web)]
fn is_content_hashed_asset(key: &str) -> bool {
    let Some(file_name) = key.strip_prefix("assets/") else {
        return false;
    };
    let Some((stem, _extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    if stem.len() <= VITE_HASH_LENGTH + 1 {
        return false;
    }
    let (name_and_separator, hash) = stem.split_at(stem.len() - VITE_HASH_LENGTH);
    name_and_separator.ends_with('-')
        && name_and_separator.len() > 1
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(raindrop_embedded_web)]
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

#[cfg(raindrop_embedded_web)]
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

fn not_found() -> Response {
    response(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        "no-store",
        Body::from("Not found"),
    )
}

#[cfg(all(test, raindrop_embedded_web))]
mod tests {
    use super::is_content_hashed_asset;

    #[test]
    fn immutable_cache_requires_an_exact_vite_hash_segment() {
        assert!(is_content_hashed_asset("assets/index-CvRIp8H1.js"));
        assert!(is_content_hashed_asset("assets/index-COyQDe_A.css"));
        assert!(is_content_hashed_asset("assets/index-DMaGHT-s.js"));
        assert!(!is_content_hashed_asset("assets/index-production.js"));
        assert!(!is_content_hashed_asset("assets/index-CvRIp8H.js"));
        assert!(!is_content_hashed_asset("assets/index-CvRIp8H12.js"));
        assert!(!is_content_hashed_asset("assets/index-CvRIp8H!.js"));
    }
}
