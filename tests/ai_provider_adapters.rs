use http::{HeaderName, HeaderValue, StatusCode};
use raindrop::content::provider::{
    EncodedProviderRequest, FinishReason, OutputSchema, ProviderAdapterErrorKind, ProviderHeader,
    ProviderKind, StructuredGenerationRequest, StructuredGenerationResponse, TokenUsage,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

const KIB: usize = 1024;

fn object_with_serialized_size(size: usize) -> Value {
    assert!(size >= 8);
    json!({ "x": "a".repeat(size - 8) })
}

fn valid_request() -> StructuredGenerationRequest {
    StructuredGenerationRequest {
        model: "model-v1".to_owned(),
        system_instruction: "Return only the requested JSON object.".to_owned(),
        untrusted_input: json!({ "article": "untrusted article text" }),
        output_schema: OutputSchema {
            name: "ai_summary_v1".to_owned(),
            schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["summary"],
                "properties": { "summary": { "type": "string" } }
            }),
        },
        max_output_tokens: 4096,
        idempotency_key: "job-request-1".to_owned(),
    }
}

#[test]
fn canonical_request_accepts_exact_upper_boundaries() {
    let request = StructuredGenerationRequest {
        model: "m".repeat(200),
        system_instruction: "s".repeat(64 * KIB),
        untrusted_input: object_with_serialized_size(512 * KIB),
        output_schema: OutputSchema {
            name: format!("a{}", "b".repeat(63)),
            schema: object_with_serialized_size(64 * KIB),
        },
        max_output_tokens: 16_384,
        idempotency_key: "i".repeat(200),
    };

    request.validate().expect("exact upper bounds should pass");
}

#[test]
fn canonical_request_rejects_invalid_bounds_without_echoing_input() {
    let cases = [
        ("empty model", {
            let mut request = valid_request();
            request.model.clear();
            request
        }),
        ("model control", {
            let mut request = valid_request();
            request.model = "model\nsecret-model".to_owned();
            request
        }),
        ("system too large", {
            let mut request = valid_request();
            request.system_instruction = "secret-system".repeat(5_042);
            request
        }),
        ("input is not object", {
            let mut request = valid_request();
            request.untrusted_input = json!(["secret-input"]);
            request
        }),
        ("input too large", {
            let mut request = valid_request();
            request.untrusted_input = object_with_serialized_size(512 * KIB + 1);
            request
        }),
        ("invalid schema name", {
            let mut request = valid_request();
            request.output_schema.name = "secret schema".to_owned();
            request
        }),
        ("schema is not object", {
            let mut request = valid_request();
            request.output_schema.schema = json!("secret-schema");
            request
        }),
        ("schema too large", {
            let mut request = valid_request();
            request.output_schema.schema = object_with_serialized_size(64 * KIB + 1);
            request
        }),
        ("tokens zero", {
            let mut request = valid_request();
            request.max_output_tokens = 0;
            request
        }),
        ("tokens too large", {
            let mut request = valid_request();
            request.max_output_tokens = 16_385;
            request
        }),
        ("idempotency control", {
            let mut request = valid_request();
            request.idempotency_key = "secret\nkey".to_owned();
            request
        }),
    ];

    for (name, request) in cases {
        let error = request.validate().expect_err(name);
        assert_eq!(
            error.kind(),
            ProviderAdapterErrorKind::InvalidRequest,
            "{name}"
        );
        let rendered = format!("{error:?} {error}");
        for secret in [
            "secret-model",
            "secret-system",
            "secret-input",
            "secret schema",
            "secret-schema",
            "secret\\nkey",
        ] {
            assert!(!rendered.contains(secret), "{name} leaked {secret}");
        }
    }
}

#[test]
fn canonical_request_rejects_oversized_encoded_body() {
    let error = EncodedProviderRequest::new(
        "/v1/test".to_owned(),
        Vec::new(),
        vec![b'x'; 1024 * KIB + 1],
    )
    .expect_err("request body over one MiB should fail");

    assert_eq!(error.kind(), ProviderAdapterErrorKind::RequestTooLarge);
}

#[test]
fn canonical_headers_and_encoded_request_debug_are_redacted() {
    let credential = "Bearer rd-secret-provider-key";
    let request_body = br#"{"prompt":"rd-secret-prompt"}"#.to_vec();
    let request = EncodedProviderRequest::new(
        "/v1/test".to_owned(),
        vec![
            ProviderHeader::public(
                HeaderName::from_static("content-type"),
                HeaderValue::from_static("application/json"),
            ),
            ProviderHeader::secret(
                HeaderName::from_static("authorization"),
                SecretString::from(credential.to_owned()),
            ),
        ],
        request_body,
    )
    .expect("bounded request should construct");

    assert_eq!(request.path(), "/v1/test");
    assert_eq!(request.body().len(), 29);
    assert!(request.headers()[1].is_secret());
    assert_eq!(request.headers()[1].name().as_str(), "authorization");
    let rendered = format!("{request:?}");
    assert!(rendered.contains("body_bytes"));
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains(credential));
    assert!(!rendered.contains("rd-secret-prompt"));
}

#[test]
fn canonical_structured_types_debug_redacts_untrusted_data() {
    let request = StructuredGenerationRequest {
        model: "public-model-label".to_owned(),
        system_instruction: "rd-secret-system-instruction".to_owned(),
        untrusted_input: json!({ "article": "rd-secret-untrusted-input" }),
        output_schema: OutputSchema {
            name: "summary_v1".to_owned(),
            schema: json!({ "description": "rd-secret-output-schema" }),
        },
        max_output_tokens: 512,
        idempotency_key: "rd-secret-idempotency-key".to_owned(),
    };
    let response = StructuredGenerationResponse {
        output: json!({ "summary": "rd-secret-model-output" }),
        finish_reason: FinishReason::Stop,
        usage: TokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(4),
        },
        model_label: "public-model-label".to_owned(),
    };

    let rendered = format!("{request:?} {response:?}");
    assert!(rendered.contains("public-model-label"));
    assert!(rendered.contains("input_bytes"));
    assert!(rendered.contains("output_bytes"));
    for secret in [
        "rd-secret-system-instruction",
        "rd-secret-untrusted-input",
        "rd-secret-output-schema",
        "rd-secret-idempotency-key",
        "rd-secret-model-output",
    ] {
        assert!(
            !rendered.contains(secret),
            "structured Debug leaked {secret}"
        );
    }
}

#[test]
fn canonical_response_limit_precedes_provider_decoding() {
    let body = vec![b'x'; 2 * 1024 * KIB + 1];
    let error = ProviderKind::OpenAiResponses
        .decode_response("model-v1", StatusCode::OK, &body)
        .expect_err("oversized body should fail before provider decoding");

    assert_eq!(error.kind(), ProviderAdapterErrorKind::ResponseTooLarge);
}

#[test]
fn request_encoders_match_v1_fixtures_and_keep_credentials_out_of_bodies() {
    let cases = [
        (
            ProviderKind::AnthropicMessages,
            "/v1/messages",
            include_str!("fixtures/ai-provider/v1/anthropic-request.json"),
            "x-api-key",
            "provider-key",
            "idempotency-key",
        ),
        (
            ProviderKind::OpenAiResponses,
            "/v1/responses",
            include_str!("fixtures/ai-provider/v1/openai-responses-request.json"),
            "authorization",
            "Bearer provider-key",
            "idempotency-key",
        ),
        (
            ProviderKind::OpenAiChatCompletions,
            "/v1/chat/completions",
            include_str!("fixtures/ai-provider/v1/openai-chat-request.json"),
            "authorization",
            "Bearer provider-key",
            "idempotency-key",
        ),
        (
            ProviderKind::GoogleGemini,
            "/v1beta/models/model-v1:generateContent",
            include_str!("fixtures/ai-provider/v1/gemini-request.json"),
            "x-goog-api-key",
            "provider-key",
            "x-goog-request-id",
        ),
    ];

    for (kind, path, fixture, secret_name, expected_secret, request_id_header) in cases {
        let encoded = kind
            .encode_request(&valid_request(), SecretString::from("provider-key"))
            .expect("provider request should encode");
        assert_eq!(encoded.path(), path, "{kind:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(encoded.body()).expect("encoded body should be JSON"),
            serde_json::from_str::<Value>(fixture).expect("fixture should be JSON"),
            "{kind:?}"
        );
        assert_eq!(public_header(&encoded, "content-type"), "application/json");
        assert_eq!(public_header(&encoded, "accept"), "application/json");
        assert_eq!(public_header(&encoded, request_id_header), "job-request-1");
        assert_eq!(secret_header(&encoded, secret_name), expected_secret);
        if kind == ProviderKind::AnthropicMessages {
            assert_eq!(public_header(&encoded, "anthropic-version"), "2023-06-01");
        }
        let body = String::from_utf8_lossy(encoded.body());
        let rendered = format!("{encoded:?}");
        assert!(
            !body.contains("provider-key"),
            "{kind:?} body leaked credential"
        );
        assert!(
            !rendered.contains("provider-key"),
            "{kind:?} Debug leaked credential"
        );
        assert!(rendered.contains("[REDACTED]"), "{kind:?}");
    }
}

#[test]
fn request_gemini_model_is_encoded_as_one_path_segment() {
    let mut request = valid_request();
    request.model = "publishers/google/models/gemini?preview#1".to_owned();

    let encoded = ProviderKind::GoogleGemini
        .encode_request(&request, SecretString::from("provider-key"))
        .expect("Gemini request should encode");

    assert_eq!(
        encoded.path(),
        "/v1beta/models/publishers%2Fgoogle%2Fmodels%2Fgemini%3Fpreview%231:generateContent"
    );
    assert!(!encoded.path().contains('?'));
    assert!(!encoded.path().contains('#'));
}

fn public_header<'a>(request: &'a EncodedProviderRequest, name: &str) -> &'a str {
    request
        .headers()
        .iter()
        .find(|header| header.name().as_str() == name)
        .and_then(ProviderHeader::public_value)
        .and_then(|value| value.to_str().ok())
        .expect("public header should exist")
}

fn secret_header<'a>(request: &'a EncodedProviderRequest, name: &str) -> &'a str {
    request
        .headers()
        .iter()
        .find(|header| header.name().as_str() == name)
        .and_then(ProviderHeader::secret_value)
        .map(ExposeSecret::expose_secret)
        .expect("secret header should exist")
}
