use std::{fmt, net::IpAddr};

use crate::feeds::{AddressDecision, AddressPolicy};

use super::{ProviderCoreError, ProviderCoreErrorKind, ProviderEndpoint, ProviderKind};

// Bound administrator-supplied exceptions and their per-request matching cost.
const MAX_TRUSTED_ENDPOINTS: usize = 32;

/// Deployment-owned exceptions for HTTPS providers on private networks.
#[derive(Clone, Default)]
pub struct ProviderNetworkPolicy {
    trusted_endpoints: Vec<ProviderEndpoint>,
}

impl ProviderNetworkPolicy {
    /// Trusts at most 32 exact HTTPS hostname base URLs for RFC 1918 and ULA addresses.
    ///
    /// # Errors
    /// Returns an error for invalid URLs, IP literals, wildcards, or an oversized list.
    pub fn new(endpoints: &[String]) -> Result<Self, ProviderCoreError> {
        if endpoints.len() > MAX_TRUSTED_ENDPOINTS {
            return Err(ProviderCoreError::new(
                ProviderCoreErrorKind::InvalidEndpoint,
            ));
        }
        let mut trusted_endpoints = Vec::new();
        for value in endpoints {
            let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiChatCompletions, Some(value))?;
            if endpoint.ip_address().is_some() || endpoint.canonical_host().contains('*') {
                return Err(ProviderCoreError::new(
                    ProviderCoreErrorKind::InvalidEndpoint,
                ));
            }
            if !trusted_endpoints.contains(&endpoint) {
                trusted_endpoints.push(endpoint);
            }
        }
        Ok(Self { trusted_endpoints })
    }

    /// Reports whether all endpoints require public addresses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trusted_endpoints.is_empty()
    }

    pub(crate) fn allows(&self, endpoint: &ProviderEndpoint, address: IpAddr) -> bool {
        if AddressPolicy::public_only().classify(address) == AddressDecision::Allowed {
            return true;
        }
        let private = match address {
            IpAddr::V4(address) => address.is_private(),
            IpAddr::V6(address) => address.is_unique_local(),
        };
        private && self.trusted_endpoints.contains(endpoint)
    }
}

impl fmt::Debug for ProviderNetworkPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderNetworkPolicy")
            .field("trusted_endpoint_count", &self.trusted_endpoints.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(value: &str) -> ProviderEndpoint {
        ProviderEndpoint::new(ProviderKind::OpenAiResponses, Some(value)).unwrap()
    }

    #[test]
    fn private_network_exceptions_require_the_exact_deployment_owned_base_url() {
        let trusted = endpoint("https://model.example/tenant-a/");
        let policy = ProviderNetworkPolicy::new(&[trusted.as_str().to_owned()]).unwrap();
        for address in ["10.0.0.8", "172.16.0.8", "192.168.1.8", "fc03::8"] {
            let address = address.parse().unwrap();
            assert!(!ProviderNetworkPolicy::default().allows(&trusted, address));
            assert!(policy.allows(&trusted, address), "{address}");
            for other in [
                "https://other.example/tenant-a/",
                "https://model.example:8443/tenant-a/",
                "https://model.example/tenant-b/",
                "https://model.example/tenant-a/child/",
            ] {
                assert!(!policy.allows(&endpoint(other), address), "{other}");
            }
        }
        assert!(policy.allows(&trusted, "93.184.216.34".parse().unwrap()));
        assert!(!format!("{policy:?}").contains("tenant-a"));
    }

    #[test]
    fn trusted_endpoints_still_reject_loopback_metadata_and_special_addresses() {
        let endpoint = endpoint("https://model.example/");
        let policy = ProviderNetworkPolicy::new(&[endpoint.as_str().to_owned()]).unwrap();
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:192.168.1.8",
            "64:ff9b::a00:8",
        ] {
            assert!(
                !policy.allows(&endpoint, address.parse().unwrap()),
                "{address}"
            );
        }
    }

    #[test]
    fn trust_configuration_validates_urls_and_normalizes_exact_matches() {
        for value in [
            "http://model.example/",
            "https://user:password@model.example/",
            "https://*.example/",
            "https://model.example/?token=private",
            "https://model.example/../secret/",
            "https://192.168.1.8/",
            "https://[fc03::8]/",
            "https://93.184.216.34/",
        ] {
            assert!(
                ProviderNetworkPolicy::new(&[value.to_owned()]).is_err(),
                "{value}"
            );
        }
        let policy =
            ProviderNetworkPolicy::new(&["https://MODEL.example:443/tenant".to_owned()]).unwrap();
        assert!(policy.allows(
            &endpoint("https://model.example/tenant/"),
            "fc03::8".parse().unwrap()
        ));
        assert!(
            ProviderNetworkPolicy::new(&vec!["https://model.example/".to_owned(); 33]).is_err()
        );
    }
}
