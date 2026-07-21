use std::time::Duration;

use async_trait::async_trait;
use dlx::{Auth, Client, SourceLang, TargetLang, TranslateRequest};
use http::{
    HeaderValue, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::content::provider::{
    EncodedProviderRequest, HttpsProviderTransport, ProviderEndpoint, ProviderHeader, ProviderKind,
    ProviderTransport, ProviderTransportErrorKind,
};

use super::{TranslationError, TranslationErrorKind};

pub(crate) const API_KEY_PLACEHOLDER: &str = "{{apiKey}}";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct DeepLxTranslateInput {
    pub base_url: Option<String>,
    pub api_key: Option<SecretString>,
    pub text: String,
    pub target_locale: String,
}

pub struct DeepLxTranslatedText {
    pub text: String,
    pub detected_source_locale: Option<String>,
}

#[async_trait]
pub trait DeepLxTransport: Send + Sync {
    async fn translate(
        &self,
        input: DeepLxTranslateInput,
    ) -> Result<DeepLxTranslatedText, TranslationError>;
}

#[derive(Default)]
pub struct ProductionDeepLxTransport;

#[async_trait]
impl DeepLxTransport for ProductionDeepLxTransport {
    async fn translate(
        &self,
        input: DeepLxTranslateInput,
    ) -> Result<DeepLxTranslatedText, TranslationError> {
        if let Some(base_url) = input.base_url.as_deref() {
            translate_custom(base_url, input.api_key, &input.text, &input.target_locale).await
        } else {
            translate_official(input.api_key, &input.text, &input.target_locale).await
        }
    }
}

async fn translate_official(
    api_key: Option<SecretString>,
    text: &str,
    target_locale: &str,
) -> Result<DeepLxTranslatedText, TranslationError> {
    let auth = api_key.map_or(Auth::Anonymous, Auth::Bearer);
    let client = Client::builder()
        .auth(auth)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(map_dlx_error)?;
    let request = TranslateRequest::builder()
        .text(text)
        .map_err(|_| invalid_input())?
        .source(SourceLang::Auto)
        .target(deeplx_target(target_locale)?)
        .build()
        .map_err(|_| invalid_input())?;
    let response = client.translate(&request).await.map_err(map_dlx_error)?;
    let translation = response
        .translations
        .into_iter()
        .next()
        .ok_or_else(|| TranslationError::new(TranslationErrorKind::Upstream))?;
    Ok(DeepLxTranslatedText {
        text: translation.text,
        detected_source_locale: response.source_lang,
    })
}

#[derive(Serialize)]
struct CustomRequest<'a> {
    text: &'a str,
    source_lang: &'static str,
    target_lang: &'a str,
}

#[derive(Deserialize)]
struct CustomResponse {
    code: Option<u16>,
    data: String,
    source_lang: Option<String>,
}

async fn translate_custom(
    base_url: &str,
    api_key: Option<SecretString>,
    text: &str,
    target_locale: &str,
) -> Result<DeepLxTranslatedText, TranslationError> {
    let expanded = expand_base_url(base_url, api_key.as_ref())?;
    let url = validate_expanded_url(&expanded)?;
    let path = url.path().to_owned();
    let mut origin = url;
    origin.set_path("/");
    let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiResponses, Some(origin.as_str()))
        .map_err(|_| invalid_input())?;
    let target = deeplx_target(target_locale)?;
    let body = serde_json::to_vec(&CustomRequest {
        text,
        source_lang: "auto",
        target_lang: target.code(),
    })
    .map_err(|_| TranslationError::new(TranslationErrorKind::Upstream))?;
    let mut headers = vec![ProviderHeader::public(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    )];
    if !base_url.contains(API_KEY_PLACEHOLDER)
        && let Some(api_key) = api_key
    {
        headers.push(ProviderHeader::secret(
            AUTHORIZATION,
            SecretString::from(format!("Bearer {}", api_key.expose_secret())),
        ));
    }
    let request = EncodedProviderRequest::new(path, headers, body).map_err(|_| invalid_input())?;
    let transport = HttpsProviderTransport::new().map_err(map_transport_error)?;
    let response = transport
        .execute("translation-deeplx", &endpoint, request)
        .await
        .map_err(map_transport_error)?;
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        return Err(TranslationError::new(TranslationErrorKind::RateLimited));
    }
    if !response.status().is_success() {
        return Err(TranslationError::new(TranslationErrorKind::Upstream));
    }
    let decoded: CustomResponse = serde_json::from_slice(response.body())
        .map_err(|_| TranslationError::new(TranslationErrorKind::Upstream))?;
    if decoded.code.is_some_and(|code| !(200..300).contains(&code))
        || !valid_upstream_text(&decoded.data, 64 * 1024)
    {
        return Err(TranslationError::new(TranslationErrorKind::Upstream));
    }
    Ok(DeepLxTranslatedText {
        text: decoded.data,
        detected_source_locale: decoded.source_lang,
    })
}

pub(crate) fn validate_base_url_template(
    value: Option<&str>,
) -> Result<Option<String>, TranslationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 2048
        || trimmed.matches(API_KEY_PLACEHOLDER).count() > 1
        || trimmed
            .replace(API_KEY_PLACEHOLDER, "")
            .contains(['{', '}'])
    {
        return Err(invalid_input());
    }
    validate_expanded_url(&trimmed.replace(API_KEY_PLACEHOLDER, "key"))?;
    Ok(Some(trimmed.to_owned()))
}

pub(crate) fn requires_url_api_key(base_url: Option<&str>) -> bool {
    base_url.is_some_and(|value| value.contains(API_KEY_PLACEHOLDER))
}

fn expand_base_url(
    base_url: &str,
    api_key: Option<&SecretString>,
) -> Result<String, TranslationError> {
    if !base_url.contains(API_KEY_PLACEHOLDER) {
        return Ok(base_url.to_owned());
    }
    let api_key =
        api_key.ok_or_else(|| TranslationError::new(TranslationErrorKind::NotConfigured))?;
    let encoded = url::form_urlencoded::byte_serialize(api_key.expose_secret().as_bytes())
        .collect::<String>();
    Ok(base_url.replace(API_KEY_PLACEHOLDER, &encoded))
}

fn validate_expanded_url(value: &str) -> Result<Url, TranslationError> {
    let url = Url::parse(value).map_err(|_| invalid_input())?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        return Err(invalid_input());
    }
    Ok(url)
}

fn deeplx_target(locale: &str) -> Result<TargetLang, TranslationError> {
    let code = match locale {
        "zh" | "zh-CN" | "zh-Hans" => "ZH-HANS",
        "zh-TW" | "zh-HK" | "zh-Hant" => "ZH-HANT",
        "en" => "EN-US",
        "pt" => "PT-BR",
        other => other,
    };
    TargetLang::parse(code).map_err(|_| invalid_input())
}

fn valid_upstream_text(value: &str, maximum_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_bytes
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn map_dlx_error(error: dlx::Error) -> TranslationError {
    let kind = match error {
        dlx::Error::TooManyRequests => TranslationErrorKind::RateLimited,
        dlx::Error::Timeout(_) => TranslationErrorKind::Timeout,
        dlx::Error::EmptyText
        | dlx::Error::TextTooLong { .. }
        | dlx::Error::MissingTargetLang
        | dlx::Error::AutoTargetLang
        | dlx::Error::UnsupportedSourceLang(_)
        | dlx::Error::UnsupportedTargetLang(_)
        | dlx::Error::InvalidHeader
        | dlx::Error::InvalidUrl
        | dlx::Error::MissingProEndpoint => TranslationErrorKind::InvalidInput,
        _ => TranslationErrorKind::Upstream,
    };
    TranslationError::new(kind)
}

fn map_transport_error(
    error: crate::content::provider::ProviderTransportError,
) -> TranslationError {
    let kind = match error.kind() {
        ProviderTransportErrorKind::Timeout => TranslationErrorKind::Timeout,
        ProviderTransportErrorKind::InvalidEndpoint
        | ProviderTransportErrorKind::AddressDenied
        | ProviderTransportErrorKind::PeerMismatch
        | ProviderTransportErrorKind::RedirectDenied => TranslationErrorKind::InvalidInput,
        _ => TranslationErrorKind::Upstream,
    };
    TranslationError::new(kind)
}

const fn invalid_input() -> TranslationError {
    TranslationError::new(TranslationErrorKind::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_key_placeholder_is_optional_but_unique() {
        assert_eq!(
            validate_base_url_template(Some("https://api.deeplx.org/{{apiKey}}/translate"))
                .unwrap(),
            Some("https://api.deeplx.org/{{apiKey}}/translate".to_owned())
        );
        assert!(validate_base_url_template(Some("https://example.com/{other}")).is_err());
        assert!(validate_base_url_template(Some("http://example.com/translate")).is_err());
    }

    #[test]
    fn url_key_is_path_encoded_and_unsafe_url_components_are_rejected() {
        let key = SecretString::from("key/with?delimiters".to_owned());
        assert_eq!(
            expand_base_url("https://api.deeplx.org/{{apiKey}}/translate", Some(&key)).unwrap(),
            "https://api.deeplx.org/key%2Fwith%3Fdelimiters/translate"
        );
        assert!(validate_base_url_template(Some("https://user@example.com/translate")).is_err());
        assert!(
            validate_base_url_template(Some("https://example.com/translate?key=value")).is_err()
        );
        assert!(
            validate_base_url_template(Some("https://example.com/translate#fragment")).is_err()
        );
    }
}
