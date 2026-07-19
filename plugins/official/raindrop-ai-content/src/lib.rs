#[cfg(any(test, target_arch = "wasm32"))]
mod config;
#[cfg(any(test, target_arch = "wasm32"))]
mod json;
#[cfg(any(test, target_arch = "wasm32"))]
mod lifecycle;
#[cfg(any(test, target_arch = "wasm32"))]
mod operation;
#[cfg(any(test, target_arch = "wasm32"))]
mod prompt;
#[cfg(any(test, target_arch = "wasm32"))]
mod tool_plan;

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Disabled,
    ConfigInvalid,
    ProviderUnavailable,
    ProviderRateLimited,
    ProviderTimeout,
    ProviderOutputInvalid,
    McpSchemaInvalid,
    McpTimeout,
    McpBudgetExhausted,
    McpRecursionBlocked,
    BudgetExhausted,
    OutputInvalid,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl Failure {
    pub(crate) const fn message_key(self) -> &'static str {
        match self {
            Self::Disabled => "raindrop.ai-content.disabled",
            Self::ConfigInvalid => "raindrop.ai-content.config-invalid",
            Self::ProviderUnavailable => "raindrop.ai-content.provider-unavailable",
            Self::ProviderRateLimited => "raindrop.ai-content.provider-rate-limited",
            Self::ProviderTimeout => "raindrop.ai-content.provider-timeout",
            Self::ProviderOutputInvalid => "raindrop.ai-content.provider-output-invalid",
            Self::McpSchemaInvalid => "raindrop.ai-content.mcp-schema-invalid",
            Self::McpTimeout => "raindrop.ai-content.mcp-timeout",
            Self::McpBudgetExhausted => "raindrop.ai-content.mcp-budget-exhausted",
            Self::McpRecursionBlocked => "raindrop.ai-content.mcp-recursion-blocked",
            Self::BudgetExhausted => "raindrop.ai-content.budget-exhausted",
            Self::OutputInvalid => "raindrop.ai-content.output-invalid",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod component;

#[cfg(test)]
mod tests {
    use super::Failure;

    #[test]
    fn every_failure_has_a_fixed_namespaced_message_key() {
        let failures = [
            Failure::Disabled,
            Failure::ConfigInvalid,
            Failure::ProviderUnavailable,
            Failure::ProviderRateLimited,
            Failure::ProviderTimeout,
            Failure::ProviderOutputInvalid,
            Failure::McpSchemaInvalid,
            Failure::McpTimeout,
            Failure::McpBudgetExhausted,
            Failure::McpRecursionBlocked,
            Failure::BudgetExhausted,
            Failure::OutputInvalid,
        ];
        for failure in failures {
            assert!(failure.message_key().starts_with("raindrop.ai-content."));
            assert!(
                failure
                    .message_key()
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'.' | b'-'))
            );
        }
    }
}
