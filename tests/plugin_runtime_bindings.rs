use raindrop::plugins::runtime::bindings::{
    ContentPluginV1,
    host_ai::GenerateRequest,
    types::{Operation, OperationRequest},
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
}
