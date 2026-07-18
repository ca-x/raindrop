use super::{
    CapabilitySession,
    bindings::{host_ai, host_mcp, types},
};

impl types::Host for CapabilitySession {}

impl host_ai::Host for CapabilitySession {
    async fn generate_structured(
        &mut self,
        request: host_ai::GenerateRequest,
    ) -> wasmtime::Result<Result<host_ai::GenerateResponse, host_ai::GenerateError>> {
        CapabilitySession::generate_structured(self, request)
            .await
            .map_err(wasmtime::Error::new)
    }
}

impl host_mcp::Host for CapabilitySession {
    async fn call_tool(
        &mut self,
        request: host_mcp::CallRequest,
    ) -> wasmtime::Result<Result<host_mcp::CallResponse, host_mcp::CallError>> {
        CapabilitySession::call_tool(self, request)
            .await
            .map_err(wasmtime::Error::new)
    }
}
