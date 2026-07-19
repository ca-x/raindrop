use std::{
    future::pending,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use raindrop::plugins::runtime::{
    AiBrokerError, AiBrokerErrorKind, AiBrokerRequest, AiBrokerResponse, AiCapabilityBroker,
    AiFinishReason, BrokerInvocationContext, CapabilitySession, CapabilitySessionConfig,
    CapabilityToolBinding, CapabilityToolBindingInput, DenyAiBroker, DenyMcpBroker, McpBrokerError,
    McpBrokerRequest, McpBrokerResponse, McpCapabilityBroker, PluginRuntimeErrorKind,
    bindings::{host_ai, host_mcp, types},
};
use serde_json::{Value, json};
use tokio::time::{Duration, Instant};

const PROVIDER_BINDING_ID: &str = "provider-binding-1";
const TOOL_PLAN_SCHEMA_ID: &str = "raindrop://schemas/plugins/raindrop.ai-content/tool-plan/v1";

#[tokio::test]
async fn ai_session_enforces_identity_before_broker_and_returns_typed_response() {
    let broker = Arc::new(RecordingAiBroker::success());
    let mut session =
        CapabilitySession::new(session_config(), broker.clone(), Arc::new(DenyMcpBroker))
            .expect("valid capability session should construct");

    let mut denied = ai_request(1);
    denied.provider_binding_id = "attacker-provider".to_owned();
    let denied = session
        .generate_structured(denied)
        .await
        .expect("validation denial should not trap")
        .expect_err("wrong provider binding should be denied");
    assert_eq!(denied.code, host_ai::GenerateErrorCode::CapabilityDenied);
    assert!(broker.requests().is_empty());

    let response = session
        .generate_structured(ai_request(1))
        .await
        .expect("broker success should not trap")
        .expect("broker should return a guest response");
    assert_eq!(response.output_json, r#"{"summary":"ok"}"#);
    assert_eq!(response.finish_reason, host_ai::FinishReason::Completed);
    assert_eq!(response.usage.input_tokens, Some(12));
    assert_eq!(response.usage.output_tokens, Some(7));
    assert_eq!(response.model_label, "test-model");
    assert_eq!(response.estimated_cost_micros, Some(42));

    let recorded = broker.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].provider_binding_id, PROVIDER_BINDING_ID);
    assert_eq!(recorded[0].operation, types::Operation::Summarize);
    assert_eq!(recorded[0].provider_request_ordinal, 1);
    assert_eq!(recorded[0].max_cost_micros, 250_000);
    assert_eq!(recorded[0].timeout, Duration::from_secs(90));

    let usage = session.usage();
    assert_eq!(usage.provider_request_count(), 1);
    assert_eq!(usage.mcp_call_count(), 0);
    assert_eq!(usage.input_tokens(), 12);
    assert_eq!(usage.output_tokens(), 7);
    assert_eq!(usage.estimated_cost_micros(), 42);
    assert!(usage.input_tokens_complete());
    assert!(usage.output_tokens_complete());
    assert!(usage.estimated_cost_complete());
    assert_eq!(usage.final_model_label(), Some("test-model"));
    assert_eq!(session.failure_hint(), None);
}

#[tokio::test]
async fn ai_session_authorizes_tool_plan_schema_from_current_host_bindings() {
    let mut config = session_config();
    config.tool_bindings = vec![tool_binding(2), tool_binding(1)];
    config.remaining_mcp_calls = 2;
    let valid_schema = tool_plan_schema(&config.tool_bindings, 2);
    let broker = Arc::new(RecordingAiBroker::success());
    let mut session = CapabilitySession::new(config, broker.clone(), Arc::new(DenyMcpBroker))
        .expect("tool-plan session should construct");
    session
        .generate_structured(tool_plan_request(valid_schema.clone()))
        .await
        .expect("tool-plan validation should not trap")
        .expect("exact host-derived schema should reach the broker");
    assert_eq!(broker.requests().len(), 1);
    assert_eq!(broker.requests()[0].output_schema_json, valid_schema);

    let exact_bindings = vec![tool_binding(1), tool_binding(2)];
    let mut binding_drift: Value =
        serde_json::from_str(&tool_plan_schema(&exact_bindings, 2)).expect("tool-plan schema JSON");
    binding_drift["properties"]["calls"]["items"]["oneOf"][1]["properties"]["toolBindingId"]["const"] =
        json!("tool-binding-9");

    let mut schema_drift: Value =
        serde_json::from_str(&tool_plan_schema(&exact_bindings, 2)).expect("tool-plan schema JSON");
    schema_drift["properties"]["calls"]["items"]["oneOf"][0]["properties"]["arguments"]["properties"]
        ["query"]["type"] = json!("integer");

    for (remaining_mcp_calls, invalid_schema) in [
        (2, canonical(binding_drift)),
        (2, canonical(schema_drift)),
        (1, tool_plan_schema(&exact_bindings, 2)),
    ] {
        let mut config = session_config();
        config.tool_bindings = exact_bindings.clone();
        config.remaining_mcp_calls = remaining_mcp_calls;
        let broker = Arc::new(RecordingAiBroker::success());
        let mut session = CapabilitySession::new(config, broker.clone(), Arc::new(DenyMcpBroker))
            .expect("drift test session should construct");
        let error = session
            .generate_structured(tool_plan_request(invalid_schema))
            .await
            .expect("schema denial should not trap")
            .expect_err("tool-plan authority drift must fail closed");
        assert_eq!(error.code, host_ai::GenerateErrorCode::InvalidRequest);
        assert!(broker.requests().is_empty());
    }
}

#[tokio::test]
async fn ai_session_enforces_operation_ordinal_json_token_and_call_budgets() {
    let cases = [
        (
            {
                let mut request = ai_request(1);
                request.operation = types::Operation::Translate;
                request
            },
            host_ai::GenerateErrorCode::CapabilityDenied,
        ),
        (ai_request(2), host_ai::GenerateErrorCode::InvalidRequest),
        (
            {
                let mut request = ai_request(1);
                request.untrusted_input_json = r#"{"text": "not-canonical"}"#.to_owned();
                request
            },
            host_ai::GenerateErrorCode::InvalidRequest,
        ),
        (
            {
                let mut request = ai_request(1);
                request.output_schema_json = "[]".to_owned();
                request
            },
            host_ai::GenerateErrorCode::InvalidRequest,
        ),
        (
            {
                let mut request = ai_request(1);
                request.system_instruction = "x".repeat(64 * 1024 + 1);
                request
            },
            host_ai::GenerateErrorCode::InvalidRequest,
        ),
        (
            {
                let mut request = ai_request(1);
                request.output_schema_json = format!(r#"{{"value":"{}"}}"#, "x".repeat(64 * 1024));
                request
            },
            host_ai::GenerateErrorCode::InvalidRequest,
        ),
        (
            {
                let mut request = ai_request(1);
                request.max_output_tokens = 4_097;
                request
            },
            host_ai::GenerateErrorCode::QuotaExceeded,
        ),
    ];

    for (request, expected) in cases {
        let broker = Arc::new(RecordingAiBroker::success());
        let mut session = session(broker.clone());
        let error = session
            .generate_structured(request)
            .await
            .expect("guest validation should not trap")
            .expect_err("invalid request should be rejected");
        assert_eq!(error.code, expected);
        assert!(broker.requests().is_empty());
    }

    let broker = Arc::new(RecordingAiBroker::success());
    let mut session = session(broker.clone());
    for ordinal in 1..=3 {
        session
            .generate_structured(ai_request(ordinal))
            .await
            .expect("broker call should not trap")
            .expect("first three calls should fit the budget");
    }
    let exhausted = session
        .generate_structured(ai_request(4))
        .await
        .expect("budget denial should not trap")
        .expect_err("fourth call should be rejected");
    assert_eq!(exhausted.code, host_ai::GenerateErrorCode::QuotaExceeded);
    assert_eq!(broker.requests().len(), 3);
    let usage = session.usage();
    assert_eq!(usage.provider_request_count(), 3);
    assert_eq!(usage.input_tokens(), 36);
    assert_eq!(usage.output_tokens(), 21);
    assert_eq!(usage.estimated_cost_micros(), 126);
    assert!(usage.input_tokens_complete());
    assert!(usage.output_tokens_complete());
    assert!(usage.estimated_cost_complete());
}

#[tokio::test]
async fn ai_session_marks_partial_host_usage_incomplete_without_trusting_guest_metrics() {
    let mut response = valid_ai_response();
    response.input_tokens = None;
    response.estimated_cost_micros = None;
    response.model_label = "partial-model".to_owned();
    let mut session = session(Arc::new(RecordingAiBroker::with_response(response)));

    session
        .generate_structured(ai_request(1))
        .await
        .expect("partial host usage should not trap")
        .expect("partial host usage should still reach the guest");

    let usage = session.usage();
    assert_eq!(usage.provider_request_count(), 1);
    assert_eq!(usage.input_tokens(), 0);
    assert_eq!(usage.output_tokens(), 7);
    assert_eq!(usage.estimated_cost_micros(), 0);
    assert!(!usage.input_tokens_complete());
    assert!(usage.output_tokens_complete());
    assert!(!usage.estimated_cost_complete());
    assert_eq!(usage.final_model_label(), Some("partial-model"));
}

#[tokio::test]
async fn ai_session_bounds_timeout_broker_errors_and_untrusted_outputs() {
    let mut timeout_config = session_config();
    timeout_config.deadline = Instant::now() + Duration::from_millis(10);
    timeout_config.deadline_unix_ms = deadline_unix_ms_after(Duration::from_millis(10));
    let mut timeout_session = CapabilitySession::new(
        timeout_config,
        Arc::new(PendingAiBroker),
        Arc::new(DenyMcpBroker),
    )
    .expect("short-deadline session should construct");
    let timed_out = timeout_session
        .generate_structured(ai_request(1))
        .await
        .expect("broker timeout should be a typed guest error")
        .expect_err("pending broker should time out");
    assert_eq!(timed_out.code, host_ai::GenerateErrorCode::Timeout);
    assert_eq!(timeout_session.usage().provider_request_count(), 1);
    assert!(!timeout_session.usage().input_tokens_complete());
    let timeout_hint = timeout_session
        .failure_hint()
        .expect("provider timeout should preserve a host failure hint");
    assert!(timeout_hint.retryable());
    assert_eq!(timeout_hint.retry_at_unix_ms(), None);
    assert!(timeout_hint.outcome_unknown());

    let broker_error = Arc::new(ErrorAiBroker(AiBrokerError::new(
        AiBrokerErrorKind::RateLimited,
        true,
        Some(1234),
    )));
    let mut broker_error_session = session(broker_error);
    let mapped = broker_error_session
        .generate_structured(ai_request(1))
        .await
        .expect("represented broker error should not trap")
        .expect_err("rate limiting should return a guest error");
    assert_eq!(mapped.code, host_ai::GenerateErrorCode::RateLimited);
    assert!(mapped.retryable);
    assert_eq!(mapped.retry_at_unix_ms, Some(1234));
    let rate_hint = broker_error_session
        .failure_hint()
        .expect("rate limit should preserve the broker hint");
    assert!(rate_hint.retryable());
    assert_eq!(rate_hint.retry_at_unix_ms(), Some(1234));
    assert!(!rate_hint.outcome_unknown());

    let mut invalid_responses = Vec::new();
    let mut noncanonical = valid_ai_response();
    noncanonical.output_json = r#"{"summary": "secret-output"}"#.to_owned();
    invalid_responses.push((
        noncanonical,
        host_ai::GenerateErrorCode::OutputSchemaInvalid,
    ));
    let mut usage = valid_ai_response();
    usage.output_tokens = Some(513);
    invalid_responses.push((usage, host_ai::GenerateErrorCode::OutputSchemaInvalid));
    let mut label = valid_ai_response();
    label.model_label = "rd-secret-model\nsecond-line".to_owned();
    invalid_responses.push((label, host_ai::GenerateErrorCode::OutputSchemaInvalid));
    let mut cost = valid_ai_response();
    cost.estimated_cost_micros = Some(250_001);
    invalid_responses.push((cost, host_ai::GenerateErrorCode::CostLimitExceeded));

    for (response, expected) in invalid_responses {
        let broker = Arc::new(RecordingAiBroker::with_response(response));
        let mut session = session(broker);
        let error = session
            .generate_structured(ai_request(1))
            .await
            .expect("invalid broker output should be typed")
            .expect_err("invalid broker output should fail closed");
        assert_eq!(error.code, expected);
        assert_eq!(session.usage().provider_request_count(), 1);
        assert!(!session.usage().input_tokens_complete());
        assert!(!format!("{error:?}").contains("secret"));
    }

    let mut failure_session = session(Arc::new(ErrorAiBroker(AiBrokerError::new(
        AiBrokerErrorKind::Failure,
        false,
        None,
    ))));
    let failure = failure_session
        .generate_structured(ai_request(1))
        .await
        .expect_err("unrepresented broker failure should trap");
    assert_eq!(failure.kind(), PluginRuntimeErrorKind::BrokerFailure);
    assert_eq!(failure_session.usage().provider_request_count(), 1);
    let failure_hint = failure_session
        .failure_hint()
        .expect("broker failure should retain a neutral host hint");
    assert!(!failure_hint.retryable());
    assert!(!failure_hint.outcome_unknown());
}

#[test]
fn capability_debug_output_redacts_identity_prompts_and_payloads() {
    let config = session_config();
    let request = AiBrokerRequest {
        provider_binding_id: "rd-secret-provider".to_owned(),
        operation: types::Operation::Summarize,
        system_instruction: "rd-secret-system".to_owned(),
        untrusted_input_json: r#"{"secret":"rd-secret-input"}"#.to_owned(),
        output_schema_id: "rd-secret-schema-id".to_owned(),
        output_schema_json: r#"{"secret":"rd-secret-schema"}"#.to_owned(),
        provider_request_ordinal: 1,
        max_input_tokens: 1,
        max_output_tokens: 1,
        max_cost_micros: 1,
        timeout: Duration::from_secs(1),
    };
    let response = AiBrokerResponse {
        output_json: r#"{"secret":"rd-secret-output"}"#.to_owned(),
        finish_reason: AiFinishReason::Completed,
        input_tokens: None,
        output_tokens: None,
        model_label: "rd-secret-model".to_owned(),
        estimated_cost_micros: None,
    };
    let mcp_request = McpBrokerRequest {
        tool_binding_id: "rd-secret-tool".to_owned(),
        arguments_json: r#"{"secret":"rd-secret-arguments"}"#.to_owned(),
        timeout: Duration::from_secs(1),
        remaining_depth: 1,
    };
    let mcp_response = McpBrokerResponse {
        result_json: r#"{"secret":"rd-secret-result"}"#.to_owned(),
        connection_label: "rd-secret-connection".to_owned(),
        tool_label: "rd-secret-tool-label".to_owned(),
    };
    let rendered = format!("{config:?} {request:?} {response:?} {mcp_request:?} {mcp_response:?}");
    assert!(!rendered.contains("rd-secret"));
}

#[test]
fn capability_session_rejects_expired_overlong_and_oversized_contexts() {
    let mut expired = session_config();
    expired.deadline_unix_ms = deadline_unix_ms_after(Duration::from_secs(1));
    expired.deadline = Instant::now() - Duration::from_millis(1);
    assert_invalid_session(expired);

    let mut overlong_automatic = session_config();
    overlong_automatic.invocation.trigger = types::Trigger::FeedRefreshPersisted;
    overlong_automatic.remaining_mcp_calls = 2;
    overlong_automatic.deadline_unix_ms = deadline_unix_ms_after(Duration::from_secs(121));
    overlong_automatic.deadline = Instant::now() + Duration::from_secs(121);
    assert_invalid_session(overlong_automatic);

    let mut too_many_tools = session_config();
    too_many_tools.tool_bindings = (0..17).map(tool_binding).collect();
    assert_invalid_session(too_many_tools);

    let mut duplicate_id = session_config();
    duplicate_id.tool_bindings.push(tool_binding(1));
    assert_invalid_session(duplicate_id);

    let mut duplicate_tool = session_config();
    let mut duplicate_tool_input = tool_binding_input(2);
    duplicate_tool_input.connection_id = "00000000-0000-4000-8000-000000000001".to_owned();
    duplicate_tool_input.tool_name = "search.read.1".to_owned();
    duplicate_tool_input.input_schema_digest =
        tool_schema_digest(&duplicate_tool_input.input_schema_json);
    duplicate_tool
        .tool_bindings
        .push(CapabilityToolBinding::new(duplicate_tool_input).expect("duplicate tool input"));
    assert_invalid_session(duplicate_tool);
}

#[test]
fn capability_tool_binding_validates_schema_identity_and_redacts_untrusted_metadata() {
    let binding = tool_binding(1);
    let wit = binding.to_wit();
    assert_eq!(wit.binding_id, "tool-binding-1");
    assert_eq!(wit.tool_name, "search.read.1");

    let invalid_inputs = [
        {
            let mut input = tool_binding_input(1);
            input.connection_id = "not-a-uuid".to_owned();
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.tool_name = "unsafe tool".to_owned();
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.display_label = "unsafe\nlabel".to_owned();
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.description = "unsafe\u{0}description".to_owned();
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.description = "x".repeat(8 * 1024 + 1);
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.display_label = "x".repeat(129);
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.input_schema_json = r#"{"type": "object"}"#.to_owned();
            input.input_schema_digest = tool_schema_digest(&input.input_schema_json);
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.input_schema_json = r#"{"type":"object","type":"array"}"#.to_owned();
            input.input_schema_digest = tool_schema_digest(&input.input_schema_json);
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.input_schema_json = format!(r#"{{"value":"{}"}}"#, "x".repeat(64 * 1024));
            input.input_schema_digest = tool_schema_digest(&input.input_schema_json);
            input
        },
        {
            let mut input = tool_binding_input(1);
            input.input_schema_digest = "a".repeat(64);
            input
        },
    ];
    for input in invalid_inputs {
        assert_eq!(
            CapabilityToolBinding::new(input)
                .expect_err("invalid tool binding must fail")
                .kind(),
            PluginRuntimeErrorKind::InvalidInvocation,
        );
    }

    let mut redacted = tool_binding_input(9);
    redacted.description = "rd-secret-description".to_owned();
    redacted.input_schema_json = r#"{"description":"rd-secret-schema","type":"object"}"#.to_owned();
    redacted.input_schema_digest = tool_schema_digest(&redacted.input_schema_json);
    let rendered = format!(
        "{:?}",
        CapabilityToolBinding::new(redacted).expect("redaction binding"),
    );
    assert!(!rendered.contains("rd-secret"));
}

#[tokio::test]
async fn mcp_session_enforces_tool_identity_and_returns_bounded_typed_response() {
    let broker = Arc::new(RecordingMcpBroker::success());
    let mut session = mcp_session(session_config(), broker.clone());

    let mut denied = mcp_request();
    denied.tool_binding_id = "attacker-tool".to_owned();
    let denied = session
        .call_tool(denied)
        .await
        .expect("tool denial should not trap")
        .expect_err("unknown tool binding should be denied");
    assert_eq!(denied.code, host_mcp::CallErrorCode::ToolDenied);
    assert!(broker.requests().is_empty());

    let response = session
        .call_tool(mcp_request())
        .await
        .expect("broker success should not trap")
        .expect("allowed tool should return a guest response");
    assert_eq!(response.result_json, r#"{"context":"ok"}"#);
    assert_eq!(response.connection_label, "test-connection");
    assert_eq!(response.tool_label, "test-tool");

    let recorded = broker.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].tool_binding_id, "tool-binding-1");
    assert_eq!(recorded[0].remaining_depth, 1);
    assert_eq!(recorded[0].timeout, Duration::from_secs(15));
    assert_eq!(session.usage().mcp_call_count(), 1);
    assert_eq!(session.failure_hint(), None);
}

#[tokio::test]
async fn mcp_session_enforces_depth_call_timeout_json_and_result_limits() {
    let invalid_requests = [
        (
            {
                let mut request = mcp_request();
                request.arguments_json = r#"{"query": "not-canonical"}"#.to_owned();
                request
            },
            host_mcp::CallErrorCode::SchemaInvalid,
        ),
        (
            {
                let mut request = mcp_request();
                request.arguments_json = "[]".to_owned();
                request
            },
            host_mcp::CallErrorCode::SchemaInvalid,
        ),
        (
            {
                let mut request = mcp_request();
                request.arguments_json = format!(r#"{{"query":"{}"}}"#, "x".repeat(64 * 1024));
                request
            },
            host_mcp::CallErrorCode::SchemaInvalid,
        ),
        (
            {
                let mut request = mcp_request();
                request.requested_timeout_ms = 0;
                request
            },
            host_mcp::CallErrorCode::Timeout,
        ),
        (
            {
                let mut request = mcp_request();
                request.requested_timeout_ms = 15_001;
                request
            },
            host_mcp::CallErrorCode::Timeout,
        ),
    ];
    for (request, expected) in invalid_requests {
        let broker = Arc::new(RecordingMcpBroker::success());
        let mut session = mcp_session(session_config(), broker.clone());
        let error = session
            .call_tool(request)
            .await
            .expect("guest validation should not trap")
            .expect_err("invalid MCP request should fail closed");
        assert_eq!(error.code, expected);
        assert!(broker.requests().is_empty());
    }

    let mut depth_config = session_config();
    depth_config.invocation.remaining_depth = 0;
    let broker = Arc::new(RecordingMcpBroker::success());
    let mut depth_session = mcp_session(depth_config, broker.clone());
    let depth = depth_session
        .call_tool(mcp_request())
        .await
        .expect("depth denial should not trap")
        .expect_err("depth zero should be rejected");
    assert_eq!(depth.code, host_mcp::CallErrorCode::RecursionBlocked);
    assert!(broker.requests().is_empty());

    let broker = Arc::new(RecordingMcpBroker::success());
    let mut call_session = mcp_session(session_config(), broker.clone());
    for _ in 0..4 {
        call_session
            .call_tool(mcp_request())
            .await
            .expect("broker call should not trap")
            .expect("first four manual calls should fit the budget");
    }
    let exhausted = call_session
        .call_tool(mcp_request())
        .await
        .expect("call budget denial should not trap")
        .expect_err("fifth manual call should be rejected");
    assert_eq!(exhausted.code, host_mcp::CallErrorCode::BudgetExhausted);
    assert_eq!(broker.requests().len(), 4);
    assert_eq!(call_session.usage().mcp_call_count(), 4);

    let mut auto_config = session_config();
    auto_config.invocation.trigger = types::Trigger::FeedRefreshPersisted;
    auto_config.remaining_mcp_calls = 3;
    let error = match CapabilitySession::new(
        auto_config,
        Arc::new(DenyAiBroker),
        Arc::new(DenyMcpBroker),
    ) {
        Ok(_) => panic!("automatic trigger must not receive three MCP calls"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), PluginRuntimeErrorKind::InvalidInvocation);
}

#[tokio::test]
async fn mcp_session_times_out_validates_results_and_defaults_to_denied() {
    let mut denied_session = mcp_session(session_config(), Arc::new(DenyMcpBroker));
    let denied = denied_session
        .call_tool(mcp_request())
        .await
        .expect("default denial should not trap")
        .expect_err("default MCP broker should deny execution");
    assert_eq!(denied.code, host_mcp::CallErrorCode::Disabled);

    let mut short_request = mcp_request();
    short_request.requested_timeout_ms = 10;
    let mut timeout_session = mcp_session(session_config(), Arc::new(PendingMcpBroker));
    let timed_out = timeout_session
        .call_tool(short_request)
        .await
        .expect("timeout should be typed")
        .expect_err("pending MCP broker should time out");
    assert_eq!(timed_out.code, host_mcp::CallErrorCode::Timeout);
    assert_eq!(timeout_session.usage().mcp_call_count(), 1);
    let timeout_hint = timeout_session
        .failure_hint()
        .expect("MCP timeout should preserve a host failure hint");
    assert!(timeout_hint.retryable());
    assert!(timeout_hint.outcome_unknown());

    let invalid_responses = [
        McpBrokerResponse {
            result_json: r#"{"context": "not-canonical"}"#.to_owned(),
            connection_label: "connection".to_owned(),
            tool_label: "tool".to_owned(),
        },
        McpBrokerResponse {
            result_json: format!(r#"{{"context":"{}"}}"#, "x".repeat(256 * 1024)),
            connection_label: "connection".to_owned(),
            tool_label: "tool".to_owned(),
        },
        McpBrokerResponse {
            result_json: r#"{"context":"ok"}"#.to_owned(),
            connection_label: "rd-secret\nsecond-line".to_owned(),
            tool_label: "tool".to_owned(),
        },
    ];
    for (index, response) in invalid_responses.into_iter().enumerate() {
        let broker = Arc::new(RecordingMcpBroker::with_response(response));
        let mut session = mcp_session(session_config(), broker);
        let error = session
            .call_tool(mcp_request())
            .await
            .expect("invalid MCP output should be typed")
            .expect_err("invalid MCP output should fail closed");
        let expected = if index == 1 {
            host_mcp::CallErrorCode::ResultTooLarge
        } else {
            host_mcp::CallErrorCode::SchemaInvalid
        };
        assert_eq!(error.code, expected);
        assert!(!format!("{error:?}").contains("secret"));
    }

    let mut mapped_session = mcp_session(
        session_config(),
        Arc::new(ErrorMcpBroker(McpBrokerError::new(
            raindrop::plugins::runtime::McpBrokerErrorKind::ConnectionDenied,
            true,
        ))),
    );
    let mapped = mapped_session
        .call_tool(mcp_request())
        .await
        .expect("represented MCP broker error should not trap")
        .expect_err("connection denial should return a guest error");
    assert_eq!(mapped.code, host_mcp::CallErrorCode::ConnectionDenied);
    assert!(mapped.retryable);
    let mapped_hint = mapped_session
        .failure_hint()
        .expect("represented MCP failure should preserve retryability");
    assert!(mapped_hint.retryable());
    assert!(!mapped_hint.outcome_unknown());

    let mut failure_session = mcp_session(
        session_config(),
        Arc::new(ErrorMcpBroker(McpBrokerError::new(
            raindrop::plugins::runtime::McpBrokerErrorKind::Failure,
            false,
        ))),
    );
    let failure = failure_session
        .call_tool(mcp_request())
        .await
        .expect_err("unrepresented MCP broker failure should trap");
    assert_eq!(failure.kind(), PluginRuntimeErrorKind::BrokerFailure);
    assert_eq!(failure_session.usage().mcp_call_count(), 1);
}

#[tokio::test]
async fn generated_host_traits_delegate_to_the_same_capability_session() {
    let ai_broker = Arc::new(RecordingAiBroker::success());
    let mcp_broker = Arc::new(RecordingMcpBroker::success());
    let mut session =
        CapabilitySession::new(session_config(), ai_broker.clone(), mcp_broker.clone())
            .expect("valid host session should construct");

    host_ai::Host::generate_structured(&mut session, ai_request(1))
        .await
        .expect("generated AI host call should not trap")
        .expect("generated AI host call should delegate");
    host_mcp::Host::call_tool(&mut session, mcp_request())
        .await
        .expect("generated MCP host call should not trap")
        .expect("generated MCP host call should delegate");

    assert_eq!(ai_broker.requests().len(), 1);
    assert_eq!(mcp_broker.requests().len(), 1);
}

fn session_config() -> CapabilitySessionConfig {
    CapabilitySessionConfig {
        invocation: BrokerInvocationContext {
            invocation_id: "invocation-1".to_owned(),
            job_id: "job-1".to_owned(),
            user_subject: "user-1".to_owned(),
            call_chain_id: "call-chain-1".to_owned(),
            operation: types::Operation::Summarize,
            trigger: types::Trigger::ManualApi,
            remaining_depth: 2,
            expected_provider_kind: "OPENAI_RESPONSES".to_owned(),
            expected_provider_model: "gpt-5-mini".to_owned(),
            expected_provider_revision: 0,
        },
        provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
        tool_bindings: vec![tool_binding(1)],
        remaining_provider_requests: 3,
        remaining_mcp_calls: 4,
        remaining_input_tokens: 8_192,
        remaining_output_tokens: 4_096,
        remaining_cost_micros: 250_000,
        deadline_unix_ms: deadline_unix_ms_after(Duration::from_secs(170)),
        deadline: Instant::now() + Duration::from_secs(170),
    }
}

fn tool_binding(index: usize) -> CapabilityToolBinding {
    CapabilityToolBinding::new(tool_binding_input(index)).expect("test tool binding")
}

fn tool_binding_input(index: usize) -> CapabilityToolBindingInput {
    let input_schema_json = r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#.to_owned();
    let input_schema_digest = tool_schema_digest(&input_schema_json);
    CapabilityToolBindingInput {
        binding_id: format!("tool-binding-{index}"),
        connection_id: format!("00000000-0000-4000-8000-{index:012}"),
        tool_name: format!("search.read.{index}"),
        display_label: format!("Search {index}"),
        description: "Untrusted search description".to_owned(),
        input_schema_json,
        input_schema_digest,
    }
}

fn tool_schema_digest(input_schema_json: &str) -> String {
    let mut hasher = blake3::Hasher::new_derive_key("raindrop.mcp-tool-input-schema.v1");
    hasher.update(&(input_schema_json.len() as u64).to_be_bytes());
    hasher.update(input_schema_json.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn deadline_unix_ms_after(duration: Duration) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow Unix epoch");
    u64::try_from((now + duration).as_millis()).expect("deadline should fit u64")
}

fn session(broker: Arc<dyn AiCapabilityBroker>) -> CapabilitySession {
    CapabilitySession::new(session_config(), broker, Arc::new(DenyMcpBroker))
        .expect("valid capability session should construct")
}

fn ai_request(ordinal: u32) -> host_ai::GenerateRequest {
    host_ai::GenerateRequest {
        provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
        operation: types::Operation::Summarize,
        system_instruction: "Summarize the untrusted input.".to_owned(),
        untrusted_input_json: r#"{"text":"untrusted"}"#.to_owned(),
        output_schema_id: "raindrop://schemas/test/summary/v1".to_owned(),
        output_schema_json: r#"{"type":"object"}"#.to_owned(),
        provider_request_ordinal: ordinal,
        max_input_tokens: 1_024,
        max_output_tokens: 512,
    }
}

fn tool_plan_request(output_schema_json: String) -> host_ai::GenerateRequest {
    host_ai::GenerateRequest {
        provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
        operation: types::Operation::Summarize,
        system_instruction: "Select bounded tools from untrusted input.".to_owned(),
        untrusted_input_json: r#"{"entry":{"text":"untrusted"}}"#.to_owned(),
        output_schema_id: TOOL_PLAN_SCHEMA_ID.to_owned(),
        output_schema_json,
        provider_request_ordinal: 1,
        max_input_tokens: 1_024,
        max_output_tokens: 1_024,
    }
}

fn tool_plan_schema(bindings: &[CapabilityToolBinding], max_calls: usize) -> String {
    let mut bindings = bindings
        .iter()
        .map(CapabilityToolBinding::to_wit)
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    let branches = bindings
        .into_iter()
        .map(|binding| {
            let input_schema: Value =
                serde_json::from_str(&binding.input_schema_json).expect("tool input schema JSON");
            json!({
                "additionalProperties": false,
                "properties": {
                    "arguments": input_schema,
                    "toolBindingId": {"const": binding.binding_id},
                },
                "required": ["toolBindingId", "arguments"],
                "type": "object",
            })
        })
        .collect::<Vec<_>>();
    canonical(json!({
        "$id": TOOL_PLAN_SCHEMA_ID,
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "additionalProperties": false,
        "properties": {
            "calls": {
                "items": {"oneOf": branches},
                "maxItems": max_calls,
                "type": "array",
            },
            "schemaVersion": {"const": 1},
        },
        "required": ["schemaVersion", "calls"],
        "type": "object",
    }))
}

fn canonical(value: Value) -> String {
    serde_json::to_string(&value).expect("canonical JSON")
}

fn mcp_request() -> host_mcp::CallRequest {
    host_mcp::CallRequest {
        tool_binding_id: "tool-binding-1".to_owned(),
        arguments_json: r#"{"query":"context"}"#.to_owned(),
        requested_timeout_ms: 15_000,
    }
}

fn mcp_session(
    config: CapabilitySessionConfig,
    broker: Arc<dyn McpCapabilityBroker>,
) -> CapabilitySession {
    CapabilitySession::new(config, Arc::new(DenyAiBroker), broker)
        .expect("valid MCP capability session should construct")
}

fn assert_invalid_session(config: CapabilitySessionConfig) {
    let error =
        match CapabilitySession::new(config, Arc::new(DenyAiBroker), Arc::new(DenyMcpBroker)) {
            Ok(_) => panic!("invalid capability session must fail closed"),
            Err(error) => error,
        };
    assert_eq!(error.kind(), PluginRuntimeErrorKind::InvalidInvocation);
}

struct RecordingAiBroker {
    requests: Mutex<Vec<AiBrokerRequest>>,
    response: AiBrokerResponse,
}

impl RecordingAiBroker {
    fn success() -> Self {
        Self::with_response(valid_ai_response())
    }

    fn with_response(response: AiBrokerResponse) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response,
        }
    }

    fn requests(&self) -> Vec<AiBrokerRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

fn valid_ai_response() -> AiBrokerResponse {
    AiBrokerResponse {
        output_json: r#"{"summary":"ok"}"#.to_owned(),
        finish_reason: AiFinishReason::Completed,
        input_tokens: Some(12),
        output_tokens: Some(7),
        model_label: "test-model".to_owned(),
        estimated_cost_micros: Some(42),
    }
}

#[async_trait]
impl AiCapabilityBroker for RecordingAiBroker {
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        assert_eq!(context.job_id, "job-1");
        self.requests.lock().expect("request lock").push(request);
        Ok(self.response.clone())
    }
}

struct PendingAiBroker;

#[async_trait]
impl AiCapabilityBroker for PendingAiBroker {
    async fn generate_structured(
        &self,
        _context: &BrokerInvocationContext,
        _request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        pending().await
    }
}

struct ErrorAiBroker(AiBrokerError);

#[async_trait]
impl AiCapabilityBroker for ErrorAiBroker {
    async fn generate_structured(
        &self,
        _context: &BrokerInvocationContext,
        _request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        Err(self.0)
    }
}

struct RecordingMcpBroker {
    requests: Mutex<Vec<McpBrokerRequest>>,
    response: McpBrokerResponse,
}

impl RecordingMcpBroker {
    fn success() -> Self {
        Self::with_response(McpBrokerResponse {
            result_json: r#"{"context":"ok"}"#.to_owned(),
            connection_label: "test-connection".to_owned(),
            tool_label: "test-tool".to_owned(),
        })
    }

    fn with_response(response: McpBrokerResponse) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response,
        }
    }

    fn requests(&self) -> Vec<McpBrokerRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait]
impl McpCapabilityBroker for RecordingMcpBroker {
    async fn call_tool(
        &self,
        _context: &BrokerInvocationContext,
        request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError> {
        self.requests.lock().expect("request lock").push(request);
        Ok(self.response.clone())
    }
}

struct PendingMcpBroker;

#[async_trait]
impl McpCapabilityBroker for PendingMcpBroker {
    async fn call_tool(
        &self,
        _context: &BrokerInvocationContext,
        _request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError> {
        pending().await
    }
}

struct ErrorMcpBroker(McpBrokerError);

#[async_trait]
impl McpCapabilityBroker for ErrorMcpBroker {
    async fn call_tool(
        &self,
        _context: &BrokerInvocationContext,
        _request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError> {
        Err(self.0)
    }
}
