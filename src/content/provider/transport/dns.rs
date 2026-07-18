use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use async_trait::async_trait;
use hickory_resolver::TokioResolver;
use tokio::time::Instant;

use crate::feeds::{AddressDecision, AddressPolicy};

use super::{
    ProviderTimeoutStage, ProviderTransportError, ProviderTransportErrorKind, strict_timeout_at,
};
use crate::content::provider::ProviderEndpoint;

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_DNS_RESULTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DnsResolveError {
    Lookup,
}

#[async_trait]
pub(super) trait DnsResolver: Send + Sync {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, DnsResolveError>;
}

pub(super) struct SystemDnsResolver {
    resolver: TokioResolver,
}

impl SystemDnsResolver {
    pub(super) fn new() -> Result<Self, hickory_resolver::net::NetError> {
        let resolver = TokioResolver::builder_tokio()?.build()?;
        Ok(Self { resolver })
    }
}

#[async_trait]
impl DnsResolver for SystemDnsResolver {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, DnsResolveError> {
        self.resolver
            .lookup_ip(host)
            .await
            .map(|lookup| lookup.iter().collect())
            .map_err(|_| DnsResolveError::Lookup)
    }
}

pub(super) async fn resolve_approved(
    provider_id: &str,
    endpoint: &ProviderEndpoint,
    resolver: &dyn DnsResolver,
    total_deadline: Instant,
) -> Result<Vec<SocketAddr>, ProviderTransportError> {
    if Instant::now() >= total_deadline {
        return Err(ProviderTransportError::timeout(
            provider_id,
            ProviderTimeoutStage::Total,
        ));
    }
    let host = endpoint.canonical_host();
    let raw = if let Ok(address) = host.parse::<IpAddr>() {
        vec![address]
    } else {
        let dns_deadline = Instant::now()
            .checked_add(DNS_TIMEOUT)
            .ok_or_else(|| ProviderTransportError::timeout(provider_id, ProviderTimeoutStage::Dns))?
            .min(total_deadline);
        strict_timeout_at(dns_deadline, resolver.resolve(host))
            .await
            .map_err(|_| {
                let stage = if Instant::now() >= total_deadline {
                    ProviderTimeoutStage::Total
                } else {
                    ProviderTimeoutStage::Dns
                };
                ProviderTransportError::timeout(provider_id, stage)
            })?
            .map_err(|_| {
                ProviderTransportError::new(provider_id, ProviderTransportErrorKind::Dns)
            })?
    };
    if raw.is_empty() || raw.len() > MAX_DNS_RESULTS {
        return Err(ProviderTransportError::new(
            provider_id,
            ProviderTransportErrorKind::AddressCount,
        )
        .with_count(raw.len()));
    }
    let policy = AddressPolicy::public_only();
    if raw
        .iter()
        .any(|address| policy.classify(*address) != AddressDecision::Allowed)
    {
        return Err(ProviderTransportError::new(
            provider_id,
            ProviderTransportErrorKind::AddressDenied,
        )
        .with_count(raw.len()));
    }
    let mut seen = HashSet::new();
    Ok(raw
        .into_iter()
        .filter(|address| seen.insert(*address))
        .map(|address| SocketAddr::new(address, endpoint.effective_port()))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::{
        future,
        net::{Ipv4Addr, Ipv6Addr},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;
    use crate::content::provider::ProviderKind;

    const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

    struct FakeResolver {
        addresses: Option<Vec<IpAddr>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DnsResolver for FakeResolver {
        async fn resolve(&self, _host: &str) -> Result<Vec<IpAddr>, DnsResolveError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.addresses {
                Some(addresses) => Ok(addresses.clone()),
                None => future::pending().await,
            }
        }
    }

    #[tokio::test]
    async fn public_answers_are_deduplicated_and_pinned_to_the_effective_port() {
        let calls = Arc::new(AtomicUsize::new(0));
        let public_v4 = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        let public_v6 = IpAddr::V6("2606:2800:220:1:248:1893:25c8:1946".parse().unwrap());
        let standard_nat64 = IpAddr::V6("64:ff9b::5db8:d822".parse().unwrap());
        let resolver = FakeResolver {
            addresses: Some(vec![public_v4, public_v4, public_v6, standard_nat64]),
            calls: calls.clone(),
        };
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example:8443/"),
        )
        .unwrap();

        let approved = resolve_approved(
            PROVIDER_ID,
            &endpoint,
            &resolver,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(approved.len(), 3);
        assert!(approved.iter().all(|address| address.port() == 8443));
    }

    #[tokio::test]
    async fn any_private_answer_and_invalid_answer_counts_fail_closed() {
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example/"),
        )
        .unwrap();
        for (addresses, expected, count) in [
            (Vec::new(), ProviderTransportErrorKind::AddressCount, 0),
            (
                (1..=17)
                    .map(|last| IpAddr::V4(Ipv4Addr::new(93, 184, 216, last)))
                    .collect(),
                ProviderTransportErrorKind::AddressCount,
                17,
            ),
            (
                vec![
                    IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                    IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                ],
                ProviderTransportErrorKind::AddressDenied,
                2,
            ),
            (
                vec![IpAddr::V6(Ipv6Addr::LOCALHOST)],
                ProviderTransportErrorKind::AddressDenied,
                1,
            ),
            (
                vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))],
                ProviderTransportErrorKind::AddressDenied,
                1,
            ),
            (
                vec![IpAddr::V6("2001:db8::7".parse().unwrap())],
                ProviderTransportErrorKind::AddressDenied,
                1,
            ),
        ] {
            let resolver = FakeResolver {
                addresses: Some(addresses),
                calls: Arc::new(AtomicUsize::new(0)),
            };
            let error = resolve_approved(
                PROVIDER_ID,
                &endpoint,
                &resolver,
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .expect_err("unsafe DNS result should fail");
            assert_eq!(error.kind(), expected);
            assert_eq!(error.count(), Some(count));
        }
    }

    #[tokio::test]
    async fn public_ip_literal_skips_dns() {
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver = FakeResolver {
            addresses: Some(vec![]),
            calls: calls.clone(),
        };
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://93.184.216.34/"),
        )
        .unwrap();

        let approved = resolve_approved(
            PROVIDER_ID,
            &endpoint,
            &resolver,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            approved,
            [SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                443,
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn resolver_is_bounded_by_the_three_second_dns_deadline() {
        let resolver = Arc::new(FakeResolver {
            addresses: None,
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example/"),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            resolve_approved(
                PROVIDER_ID,
                &endpoint,
                resolver.as_ref(),
                Instant::now() + Duration::from_secs(30),
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;

        let error = task
            .await
            .unwrap()
            .expect_err("pending DNS should time out");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::Dns));
    }

    #[tokio::test(start_paused = true)]
    async fn total_deadline_precedes_dns_deadline_and_lookup_errors_are_normalized() {
        let resolver = Arc::new(FakeResolver {
            addresses: None,
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example/"),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            resolve_approved(
                PROVIDER_ID,
                &endpoint,
                resolver.as_ref(),
                Instant::now() + Duration::from_secs(2),
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        let error = task.await.unwrap().expect_err("total deadline should win");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Timeout);
        assert_eq!(error.stage(), Some(ProviderTimeoutStage::Total));

        struct ErrorResolver;

        #[async_trait]
        impl DnsResolver for ErrorResolver {
            async fn resolve(&self, _host: &str) -> Result<Vec<IpAddr>, DnsResolveError> {
                Err(DnsResolveError::Lookup)
            }
        }

        let endpoint = ProviderEndpoint::new(
            ProviderKind::OpenAiResponses,
            Some("https://provider.example/"),
        )
        .unwrap();
        let error = resolve_approved(
            PROVIDER_ID,
            &endpoint,
            &ErrorResolver,
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .expect_err("lookup failure should be normalized");
        assert_eq!(error.kind(), ProviderTransportErrorKind::Dns);
        assert_eq!(error.stage(), None);
    }
}
