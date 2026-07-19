use crate::content::provider::ProviderPolicy;

pub(super) fn estimate_cost_micros(
    policy: ProviderPolicy,
    input_tokens: u64,
    output_tokens: u64,
) -> Option<u64> {
    let input_rate = policy.input_cost_micros_per_million_tokens?;
    let output_rate = policy.output_cost_micros_per_million_tokens?;
    token_cost(input_tokens, input_rate)?.checked_add(token_cost(output_tokens, output_rate)?)
}

fn token_cost(tokens: u64, rate: u64) -> Option<u64> {
    let numerator = u128::from(tokens).checked_mul(u128::from(rate))?;
    u64::try_from(numerator.div_ceil(1_000_000)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_estimation_is_checked_ceil_and_requires_complete_pricing() {
        let priced = ProviderPolicy {
            max_concurrency: 1,
            requests_per_minute: None,
            max_input_tokens_per_request: 10_000,
            max_output_tokens_per_request: 10_000,
            input_cost_micros_per_million_tokens: Some(1_000),
            output_cost_micros_per_million_tokens: Some(2_000),
            max_cost_micros_per_request: None,
        };
        assert_eq!(estimate_cost_micros(priced, 1_500, 500), Some(3));
        assert_eq!(estimate_cost_micros(priced, 1, 1), Some(2));

        let incomplete = ProviderPolicy {
            input_cost_micros_per_million_tokens: None,
            ..priced
        };
        assert_eq!(estimate_cost_micros(incomplete, 1_500, 500), None);
    }
}
