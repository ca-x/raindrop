use raindrop::plugins::runtime::bindings::{
    host_ai::GenerateRequest,
    types::{Operation, OperationRequest},
};

#[test]
fn native_protocol_shares_operation_types_with_the_official_service() {
    fn host_operation(request: &GenerateRequest) -> Operation {
        request.operation
    }
    fn plugin_operation(request: &OperationRequest) -> Operation {
        request.operation
    }
    let _: fn(&GenerateRequest) -> Operation = host_operation;
    let _: fn(&OperationRequest) -> Operation = plugin_operation;
    assert_eq!(
        raindrop_ai_content::AiContentService::descriptor().abi,
        raindrop_ai_content::ABI
    );
}
