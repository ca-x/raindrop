use std::{
    future::pending,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use raindrop::plugins::runtime::{
    AiBrokerError, AiBrokerErrorKind, AiBrokerRequest, AiBrokerResponse, AiCapabilityBroker,
    AiFinishReason, BrokerInvocationContext, CapabilitySession, CapabilitySessionConfig,
    DenyAiBroker, DenyMcpBroker, McpBrokerError, McpBrokerRequest, McpBrokerResponse,
    McpCapabilityBroker, PluginRuntimeErrorKind,
    bindings::{host_ai, host_mcp, types},
};
use tokio::time::{Duration, Instant};

const PROVIDER_BINDING_ID: &str = "provider-binding-1";

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
    too_many_tools.tool_binding_ids = (0..17).map(|index| format!("tool-{index}")).collect();
    assert_invalid_session(too_many_tools);
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
        },
        provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
        tool_binding_ids: vec!["tool-binding-1".to_owned()],
        remaining_provider_requests: 3,
        remaining_mcp_calls: 4,
        remaining_input_tokens: 8_192,
        remaining_output_tokens: 4_096,
        remaining_cost_micros: 250_000,
        deadline_unix_ms: deadline_unix_ms_after(Duration::from_secs(170)),
        deadline: Instant::now() + Duration::from_secs(170),
    }
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
