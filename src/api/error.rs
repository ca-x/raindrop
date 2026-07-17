use std::collections::BTreeMap;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ApiErrorEnvelope {
    pub error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
    pub request_id: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code,
                message,
                fields: None,
                request_id: Uuid::new_v4().to_string(),
            },
        }
    }

    #[must_use]
    pub fn with_field(mut self, field: impl Into<String>, message: impl Into<String>) -> Self {
        self.body
            .fields
            .get_or_insert_with(BTreeMap::new)
            .insert(field.into(), message.into());
        self
    }

    #[must_use]
    pub fn setup_token_required() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "SETUP_TOKEN_REQUIRED",
            "A valid setup token is required",
        )
    }

    #[must_use]
    pub fn database_url_invalid() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Request validation failed",
        )
        .with_field("databaseUrl", "Database URL is invalid or unavailable")
    }

    #[must_use]
    pub fn validation() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Request validation failed",
        )
    }

    #[must_use]
    pub fn setup_already_complete() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "SETUP_ALREADY_COMPLETE",
            "Initial setup has already been completed",
        )
    }

    #[must_use]
    pub fn setup_required() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "SETUP_REQUIRED",
            "Initial setup must be completed first",
        )
    }

    #[must_use]
    pub fn authentication_required() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "AUTHENTICATION_REQUIRED",
            "Authentication is required",
        )
    }

    #[must_use]
    pub fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "Invalid username, email, or password",
        )
    }

    #[must_use]
    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "The request is not allowed",
        )
    }

    #[must_use]
    pub fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "Too many requests; try again later",
        )
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", "Resource not found")
    }

    #[must_use]
    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "The request method is not allowed",
        )
    }

    #[must_use]
    pub fn username_invalid(message: &'static str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Request validation failed",
        )
        .with_field("username", message)
    }

    #[must_use]
    pub fn password_invalid() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Request validation failed",
        )
        .with_field("password", "Password must contain at least 12 bytes")
    }

    #[must_use]
    pub fn email_invalid() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Request validation failed",
        )
        .with_field("email", "Email address is invalid")
    }

    #[must_use]
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            "The request could not be completed",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ApiErrorEnvelope { error: self.body })).into_response()
    }
}
