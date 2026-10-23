use async_trait::async_trait;
use raindrop_ai_content::api::ContentHost;

use super::{
    CapabilitySession, PluginRuntimeError,
    bindings::{host_ai, host_mcp},
};

pub(super) struct NativeHost<'a> {
    pub session: &'a mut CapabilitySession,
    pub error: Option<PluginRuntimeError>,
}

#[async_trait]
impl ContentHost for NativeHost<'_> {
    async fn generate_structured(
        &mut self,
        request: host_ai::GenerateRequest,
    ) -> Result<Result<host_ai::GenerateResponse, host_ai::GenerateError>, ()> {
        self.session
            .generate_structured(request)
            .await
            .map_err(|error| {
                self.error = Some(error);
            })
    }

    async fn call_tool(
        &mut self,
        request: host_mcp::CallRequest,
    ) -> Result<Result<host_mcp::CallResponse, host_mcp::CallError>, ()> {
        self.session.call_tool(request).await.map_err(|error| {
            self.error = Some(error);
        })
    }
}
