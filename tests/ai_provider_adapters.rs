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

#[test]
fn response_decoders_match_v1_fixtures() {
    let cases = [
        (
            ProviderKind::AnthropicMessages,
            include_str!("fixtures/ai-provider/v1/anthropic-response.json"),
            "claude-fixture",
            12,
            5,
        ),
        (
            ProviderKind::OpenAiResponses,
            include_str!("fixtures/ai-provider/v1/openai-responses-response.json"),
            "gpt-responses-fixture",
            14,
            6,
        ),
        (
            ProviderKind::OpenAiChatCompletions,
            include_str!("fixtures/ai-provider/v1/openai-chat-response.json"),
            "gpt-chat-fixture",
            16,
            7,
        ),
        (
            ProviderKind::GoogleGemini,
            include_str!("fixtures/ai-provider/v1/gemini-response.json"),
            "gemini-fixture",
            18,
            8,
        ),
    ];

    for (kind, fixture, model, input_tokens, output_tokens) in cases {
        let response = kind
            .decode_response("requested-model", StatusCode::OK, fixture.as_bytes())
            .expect("fixture should decode");
        assert_eq!(
            response.output,
            json!({ "summary": "fixture summary" }),
            "{kind:?}"
        );
        assert_eq!(response.finish_reason, FinishReason::Stop, "{kind:?}");
        assert_eq!(response.model_label, model, "{kind:?}");
        assert_eq!(
            response.usage,
            TokenUsage {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            },
            "{kind:?}"
        );
    }
}

#[test]
fn response_decoders_preserve_missing_and_zero_usage_and_model_fallback() {
    for kind in provider_kinds() {
        let mut missing = response_fixture_value(kind);
        remove_usage(kind, &mut missing);
        remove_model(kind, &mut missing);
        let response = decode_value(kind, "requested-fallback", &missing).expect("missing usage");
        assert_eq!(response.usage, TokenUsage::default(), "{kind:?}");
        assert_eq!(response.model_label, "requested-fallback", "{kind:?}");

        let mut zero = response_fixture_value(kind);
        set_usage(kind, &mut zero, 0, 0);
        let response = decode_value(kind, "requested-model", &zero).expect("zero usage");
        assert_eq!(
            response.usage,
            TokenUsage {
                input_tokens: Some(0),
                output_tokens: Some(0),
            },
            "{kind:?}"
        );
    }
}

#[test]
fn response_finish_reasons_normalize_each_provider_family() {
    let anthropic = response_fixture_value(ProviderKind::AnthropicMessages);
    for (value, expected) in [
        ("end_turn", FinishReason::Stop),
        ("stop_sequence", FinishReason::Stop),
        ("max_tokens", FinishReason::Length),
        ("refusal", FinishReason::ContentFilter),
        ("tool_use", FinishReason::ToolCall),
        ("future_reason", FinishReason::Other),
    ] {
        let mut fixture = anthropic.clone();
        fixture["stop_reason"] = json!(value);
        assert_finish(ProviderKind::AnthropicMessages, fixture, expected);
    }

    let responses = response_fixture_value(ProviderKind::OpenAiResponses);
    assert_finish(
        ProviderKind::OpenAiResponses,
        responses.clone(),
        FinishReason::Stop,
    );
    for (reason, expected) in [
        ("max_output_tokens", FinishReason::Length),
        ("content_filter", FinishReason::ContentFilter),
        ("tool_call", FinishReason::ToolCall),
        ("future_reason", FinishReason::Other),
    ] {
        let mut fixture = responses.clone();
        fixture["status"] = json!("incomplete");
        fixture["incomplete_details"] = json!({ "reason": reason });
        assert_finish(ProviderKind::OpenAiResponses, fixture, expected);
    }

    let chat = response_fixture_value(ProviderKind::OpenAiChatCompletions);
    for (reason, expected) in [
        ("stop", FinishReason::Stop),
        ("length", FinishReason::Length),
        ("content_filter", FinishReason::ContentFilter),
        ("tool_calls", FinishReason::ToolCall),
        ("function_call", FinishReason::ToolCall),
        ("future_reason", FinishReason::Other),
    ] {
        let mut fixture = chat.clone();
        fixture["choices"][0]["finish_reason"] = json!(reason);
        assert_finish(ProviderKind::OpenAiChatCompletions, fixture, expected);
    }

    let gemini = response_fixture_value(ProviderKind::GoogleGemini);
    for (reason, expected) in [
        ("STOP", FinishReason::Stop),
        ("MAX_TOKENS", FinishReason::Length),
        ("SAFETY", FinishReason::ContentFilter),
        ("RECITATION", FinishReason::ContentFilter),
        ("MALFORMED_FUNCTION_CALL", FinishReason::ToolCall),
        ("UNEXPECTED_TOOL_CALL", FinishReason::ToolCall),
        ("FUTURE_REASON", FinishReason::Other),
    ] {
        let mut fixture = gemini.clone();
        fixture["candidates"][0]["finishReason"] = json!(reason);
        assert_finish(ProviderKind::GoogleGemini, fixture, expected);
    }
}

#[test]
fn response_status_errors_are_stable_and_discard_upstream_bodies() {
    let secret_body = br#"{"error":{"message":"rd-secret-provider-body and rd-secret-prompt"}}"#;
    let cases = [
        (
            StatusCode::UNAUTHORIZED,
            ProviderAdapterErrorKind::Authentication,
        ),
        (
            StatusCode::FORBIDDEN,
            ProviderAdapterErrorKind::Authentication,
        ),
        (
            StatusCode::REQUEST_TIMEOUT,
            ProviderAdapterErrorKind::Timeout,
        ),
        (
            StatusCode::GATEWAY_TIMEOUT,
            ProviderAdapterErrorKind::Timeout,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            ProviderAdapterErrorKind::RateLimited,
        ),
        (StatusCode::BAD_REQUEST, ProviderAdapterErrorKind::Rejected),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            ProviderAdapterErrorKind::Upstream,
        ),
    ];

    for kind in provider_kinds() {
        for (status, expected) in cases {
            let error = kind
                .decode_response("requested-model", status, secret_body)
                .expect_err("non-success status should fail");
            assert_eq!(error.provider(), Some(kind));
            assert_eq!(error.kind(), expected, "{kind:?} {status}");
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains("rd-secret-provider-body"));
            assert!(!rendered.contains("rd-secret-prompt"));
        }
    }
}

#[test]
fn response_decoders_fail_closed_on_ambiguous_or_invalid_output() {
    for kind in provider_kinds() {
        let mut missing = response_fixture_value(kind);
        clear_output_text(kind, &mut missing);
        assert_response_error(kind, missing, ProviderAdapterErrorKind::MalformedResponse);

        let mut multiple = response_fixture_value(kind);
        duplicate_output_text(kind, &mut multiple);
        assert_response_error(kind, multiple, ProviderAdapterErrorKind::MalformedResponse);
    }

    let mut non_object = response_fixture_value(ProviderKind::OpenAiChatCompletions);
    non_object["choices"][0]["message"]["content"] = json!("[1,2,3]");
    assert_response_error(
        ProviderKind::OpenAiChatCompletions,
        non_object,
        ProviderAdapterErrorKind::OutputSchemaInvalid,
    );

    let exact_output = object_with_serialized_size(512 * KIB);
    let mut exact = response_fixture_value(ProviderKind::OpenAiChatCompletions);
    exact["choices"][0]["message"]["content"] = json!(exact_output.to_string());
    let response = decode_value(
        ProviderKind::OpenAiChatCompletions,
        "requested-model",
        &exact,
    )
    .expect("exact output cap should pass");
    assert_eq!(response.output, exact_output);

    let mut oversized = response_fixture_value(ProviderKind::OpenAiChatCompletions);
    oversized["choices"][0]["message"]["content"] =
        json!(object_with_serialized_size(512 * KIB + 1).to_string());
    assert_response_error(
        ProviderKind::OpenAiChatCompletions,
        oversized,
        ProviderAdapterErrorKind::OutputSchemaInvalid,
    );

    let nested = format!("{}{{}}{}", "{\"x\":".repeat(140), "}".repeat(140));
    let mut excessive_depth = response_fixture_value(ProviderKind::OpenAiChatCompletions);
    excessive_depth["choices"][0]["message"]["content"] = json!(nested);
    assert_response_error(
        ProviderKind::OpenAiChatCompletions,
        excessive_depth,
        ProviderAdapterErrorKind::OutputSchemaInvalid,
    );

    let error = ProviderKind::GoogleGemini
        .decode_response("requested-model", StatusCode::OK, br#"{"#)
        .expect_err("malformed provider JSON should fail");
    assert_eq!(error.kind(), ProviderAdapterErrorKind::MalformedResponse);

    for kind in provider_kinds() {
        let mut invalid_model = response_fixture_value(kind);
        set_model(kind, &mut invalid_model, "model\nrd-secret-model-label");
        let error = decode_value(kind, "requested-model", &invalid_model)
            .expect_err("invalid response model should fail");
        assert_eq!(error.kind(), ProviderAdapterErrorKind::MalformedResponse);
        assert!(!format!("{error:?} {error}").contains("rd-secret-model-label"));
    }
}

fn provider_kinds() -> [ProviderKind; 4] {
    [
        ProviderKind::AnthropicMessages,
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiChatCompletions,
        ProviderKind::GoogleGemini,
    ]
}

fn response_fixture_value(kind: ProviderKind) -> Value {
    let fixture = match kind {
        ProviderKind::AnthropicMessages => {
            include_str!("fixtures/ai-provider/v1/anthropic-response.json")
        }
        ProviderKind::OpenAiResponses => {
            include_str!("fixtures/ai-provider/v1/openai-responses-response.json")
        }
        ProviderKind::OpenAiChatCompletions => {
            include_str!("fixtures/ai-provider/v1/openai-chat-response.json")
        }
        ProviderKind::GoogleGemini => {
            include_str!("fixtures/ai-provider/v1/gemini-response.json")
        }
    };
    serde_json::from_str(fixture).expect("response fixture should be JSON")
}

fn decode_value(
    kind: ProviderKind,
    requested_model: &str,
    value: &Value,
) -> Result<StructuredGenerationResponse, raindrop::content::provider::ProviderAdapterError> {
    kind.decode_response(
        requested_model,
        StatusCode::OK,
        &serde_json::to_vec(value).expect("fixture value should encode"),
    )
}

fn assert_finish(kind: ProviderKind, fixture: Value, expected: FinishReason) {
    let response = decode_value(kind, "requested-model", &fixture).expect("fixture should decode");
    assert_eq!(response.finish_reason, expected, "{kind:?}");
}

fn assert_response_error(kind: ProviderKind, fixture: Value, expected: ProviderAdapterErrorKind) {
    let error = decode_value(kind, "requested-model", &fixture).expect_err("fixture should fail");
    assert_eq!(error.kind(), expected, "{kind:?}");
}

fn remove_usage(kind: ProviderKind, fixture: &mut Value) {
    fixture
        .as_object_mut()
        .expect("fixture should be an object")
        .remove(match kind {
            ProviderKind::GoogleGemini => "usageMetadata",
            _ => "usage",
        });
}

fn remove_model(kind: ProviderKind, fixture: &mut Value) {
    fixture
        .as_object_mut()
        .expect("fixture should be an object")
        .remove(match kind {
            ProviderKind::GoogleGemini => "modelVersion",
            _ => "model",
        });
}

fn set_usage(kind: ProviderKind, fixture: &mut Value, input: u64, output: u64) {
    match kind {
        ProviderKind::AnthropicMessages => {
            fixture["usage"] = json!({ "input_tokens": input, "output_tokens": output });
        }
        ProviderKind::OpenAiResponses => {
            fixture["usage"] = json!({ "input_tokens": input, "output_tokens": output });
        }
        ProviderKind::OpenAiChatCompletions => {
            fixture["usage"] = json!({ "prompt_tokens": input, "completion_tokens": output });
        }
        ProviderKind::GoogleGemini => {
            fixture["usageMetadata"] =
                json!({ "promptTokenCount": input, "candidatesTokenCount": output });
        }
    }
}

fn set_model(kind: ProviderKind, fixture: &mut Value, model: &str) {
    fixture[match kind {
        ProviderKind::GoogleGemini => "modelVersion",
        _ => "model",
    }] = json!(model);
}

fn clear_output_text(kind: ProviderKind, fixture: &mut Value) {
    match kind {
        ProviderKind::AnthropicMessages => fixture["content"] = json!([]),
        ProviderKind::OpenAiResponses => fixture["output"][0]["content"] = json!([]),
        ProviderKind::OpenAiChatCompletions => fixture["choices"] = json!([]),
        ProviderKind::GoogleGemini => fixture["candidates"][0]["content"]["parts"] = json!([]),
    }
}

fn duplicate_output_text(kind: ProviderKind, fixture: &mut Value) {
    match kind {
        ProviderKind::AnthropicMessages => {
            let duplicate = fixture["content"][0].clone();
            fixture["content"].as_array_mut().unwrap().push(duplicate);
        }
        ProviderKind::OpenAiResponses => {
            let duplicate = fixture["output"][0]["content"][0].clone();
            fixture["output"][0]["content"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
        }
        ProviderKind::OpenAiChatCompletions => {
            let duplicate = fixture["choices"][0].clone();
            fixture["choices"].as_array_mut().unwrap().push(duplicate);
        }
        ProviderKind::GoogleGemini => {
            let duplicate = fixture["candidates"][0]["content"]["parts"][0].clone();
            fixture["candidates"][0]["content"]["parts"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
        }
    }
}
