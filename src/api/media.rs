use axum::{
    body::Body,
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, PRAGMA, VARY, X_CONTENT_TYPE_OPTIONS},
    },
    response::Response,
};
use tokio::sync::TryAcquireError;

use crate::{
    app::AppState,
    feeds::{FeedUrlPolicy, FetchOutcome, FetchRequest},
};

use super::ApiError;

pub(super) async fn media_cache_headers(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Cookie"));
    if !response.status().is_success() || !response.headers().contains_key(CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    }
    response
}

pub(super) async fn fetch_raster_image(
    state: &AppState,
    source_url: &str,
    max_bytes: usize,
    cache_control: &'static str,
) -> Result<Response, ApiError> {
    let url = FeedUrlPolicy::new(false)
        .normalize(source_url)
        .map_err(|_| ApiError::not_found())?;
    let _permit = match state.media_fetch_semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(TryAcquireError::NoPermits) => return Err(ApiError::rate_limited()),
        Err(TryAcquireError::Closed) => return Err(ApiError::internal()),
    };
    let transport = state.media_transport().ok_or_else(ApiError::internal)?;
    let outcome = transport
        .fetch(FetchRequest::new(url, None))
        .await
        .map_err(|_| ApiError::not_found())?;
    let FetchOutcome::Document { document, .. } = outcome else {
        return Err(ApiError::not_found());
    };
    raster_image_response(document, max_bytes, cache_control)
}

fn raster_image_response(
    document: Vec<u8>,
    max_bytes: usize,
    cache_control: &'static str,
) -> Result<Response, ApiError> {
    if document.is_empty() || document.len() > max_bytes {
        return Err(ApiError::not_found());
    }
    let content_type = raster_image_content_type(&document).ok_or_else(ApiError::not_found)?;
    let mut response = Response::new(Body::from(document));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

fn raster_image_content_type(document: &[u8]) -> Option<&'static str> {
    if document.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        Some("image/x-icon")
    } else if document.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if document.starts_with(b"GIF87a") || document.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if document.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if document.len() >= 12 && document.starts_with(b"RIFF") && &document[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    use super::*;

    #[test]
    fn raster_image_sniffing_rejects_active_content() {
        assert_eq!(
            raster_image_content_type(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(raster_image_content_type(b"GIF89arest"), Some("image/gif"));
        assert_eq!(
            raster_image_content_type(b"<svg onload='evil()'></svg>"),
            None
        );
        assert_eq!(
            raster_image_content_type(b"<html>not an image</html>"),
            None
        );
    }

    #[tokio::test]
    async fn media_fetch_rejects_when_capacity_is_exhausted() {
        let mut state = AppState::for_test();
        state.media_fetch_semaphore = Arc::new(Semaphore::new(0));

        let error = fetch_raster_image(
            &state,
            "https://images.example.test/preview.png",
            1024,
            "private, no-cache, max-age=0, must-revalidate",
        )
        .await
        .expect_err("an exhausted media fetch pool must reject immediately");

        assert_eq!(
            error.into_response().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
