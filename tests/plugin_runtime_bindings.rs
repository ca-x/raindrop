use raindrop::plugins::runtime::bindings::{
    ContentPluginV1,
    host_ai::GenerateRequest,
    types::{
        LifecycleEvent, LifecycleRequest, Operation, OperationRequest, ToolBinding, UserScope,
    },
};

#[test]
fn generated_world_shares_operation_types_across_host_and_guest() {
    fn host_operation(request: &GenerateRequest) -> &Operation {
        &request.operation
    }

    fn guest_operation(request: &OperationRequest) -> &Operation {
        &request.operation
    }

    let _generated_world_type = std::mem::size_of::<ContentPluginV1>();
    let _host_type_identity: fn(&GenerateRequest) -> &Operation = host_operation;
    let _guest_type_identity: fn(&OperationRequest) -> &Operation = guest_operation;

    let binding = ToolBinding {
        binding_id: "binding-1".to_owned(),
        connection_id: "00000000-0000-4000-8000-000000000001".to_owned(),
        tool_name: "search.read".to_owned(),
        display_label: "Search".to_owned(),
        description: "Untrusted tool description".to_owned(),
        input_schema_json: r#"{"type":"object"}"#.to_owned(),
        input_schema_digest: "a".repeat(64),
    };
    assert_eq!(binding.tool_name, "search.read");

    let lifecycle = LifecycleRequest {
        invocation_id: "invocation-1".to_owned(),
        plugin_key: "raindrop.ai-content".to_owned(),
        plugin_version: "1.0.0".to_owned(),
        component_digest: "b".repeat(64),
        config_json: "{}".to_owned(),
        config_hash: "c".repeat(64),
        event: LifecycleEvent {
            event_id: "event-1".to_owned(),
            event_type: "feed.refresh.persisted".to_owned(),
            schema_version: 1,
            refresh_id: "refresh-1".to_owned(),
            sequence: 10,
            occurred_at: "2026-07-19T00:00:00Z".to_owned(),
            idempotency_key: "refresh:refresh-1:persisted:v1".to_owned(),
            user_scope: UserScope {
                subject: "user-1".to_owned(),
            },
            context_json: "{}".to_owned(),
        },
    };
    assert_eq!(lifecycle.plugin_key, "raindrop.ai-content");
}
