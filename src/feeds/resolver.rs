use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};

use async_trait::async_trait;
use hickory_resolver::{
    TokioResolver,
    config::ResolveHosts,
    lookup::Lookup,
    net::{DnsError, NetError},
    proto::{
        op::ResponseCode,
        rr::{DNSClass, RData, RecordType},
    },
};
use ipnet::Ipv6Net;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{Instant, timeout_at};

use super::{AddressPolicy, AddressPolicyError};

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const IPV4ONLY_ARPA: &str = "ipv4only.arpa.";
const WKA_170: Ipv4Addr = Ipv4Addr::new(192, 0, 0, 170);
const WKA_171: Ipv4Addr = Ipv4Addr::new(192, 0, 0, 171);
const SUPPORTED_PREFIX_LENGTHS: [u8; 6] = [32, 40, 48, 56, 64, 96];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DnsResolveError {
    Deadline,
    Lookup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum Nat64DiscoveryError {
    #[error("NAT64 discovery deadline expired")]
    Deadline,
    #[error("NAT64 discovery lookup failed")]
    Lookup,
    #[error("NAT64 discovery A response is not verified")]
    MissingWellKnownAddresses,
    #[error("NAT64 discovery AAAA negative response is not verified")]
    InvalidNegativeResponse,
    #[error("NAT64 discovery TTL is invalid")]
    InvalidTtl,
    #[error("NAT64 discovery answer is malformed")]
    MalformedAnswer,
    #[error("NAT64 discovery answer is ambiguous")]
    AmbiguousAnswer,
    #[error("NAT64 discovery prefix is invalid")]
    InvalidPrefix,
}

impl From<AddressPolicyError> for Nat64DiscoveryError {
    fn from(_: AddressPolicyError) -> Self {
        Self::InvalidPrefix
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Nat64DiscoveryState {
    Present(Vec<Ipv6Net>),
    NotPresent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Nat64Discovery {
    pub(super) state: Nat64DiscoveryState,
    pub(super) valid_until: Instant,
}

#[async_trait]
pub(super) trait DnsResolver: Send + Sync {
    async fn resolve(&self, host: &str, deadline: Instant) -> Result<Vec<IpAddr>, DnsResolveError>;
}

#[async_trait]
pub(super) trait Nat64PrefixDiscovery: Send + Sync {
    async fn discover(&self, deadline: Instant) -> Result<Nat64Discovery, Nat64DiscoveryError>;
}

#[async_trait]
trait ExplicitLookup: Send + Sync {
    async fn lookup(&self, name: &str, record_type: RecordType) -> Result<Lookup, NetError>;
}

#[async_trait]
impl ExplicitLookup for TokioResolver {
    async fn lookup(&self, name: &str, record_type: RecordType) -> Result<Lookup, NetError> {
        self.lookup(name, record_type).await
    }
}

pub(super) struct SystemDnsResolver {
    lookup: Arc<dyn ExplicitLookup>,
}

impl SystemDnsResolver {
    pub(super) fn new() -> Result<Self, NetError> {
        let resolver = TokioResolver::builder_tokio()?.build()?;
        Ok(Self {
            lookup: Arc::new(resolver),
        })
    }
}

#[async_trait]
impl DnsResolver for SystemDnsResolver {
    async fn resolve(&self, host: &str, deadline: Instant) -> Result<Vec<IpAddr>, DnsResolveError> {
        if Instant::now() >= deadline {
            return Err(DnsResolveError::Deadline);
        }
        let fqdn = absolute_name(host);
        let lookup = self.lookup.clone();
        let pair = timeout_at(deadline, async move {
            tokio::join!(
                lookup.lookup(&fqdn, RecordType::A),
                lookup.lookup(&fqdn, RecordType::AAAA)
            )
        })
        .await
        .map_err(|_| DnsResolveError::Deadline)?;

        let mut addresses = Vec::new();
        collect_family_addresses(pair.0, RecordType::A, &mut addresses)?;
        collect_family_addresses(pair.1, RecordType::AAAA, &mut addresses)?;
        Ok(addresses)
    }
}

fn collect_family_addresses(
    result: Result<Lookup, NetError>,
    record_type: RecordType,
    addresses: &mut Vec<IpAddr>,
) -> Result<(), DnsResolveError> {
    match result {
        Ok(lookup) => {
            for record in lookup.answers() {
                match (&record.data, record_type) {
                    (RData::A(address), RecordType::A) => addresses.push(IpAddr::V4(address.0)),
                    (RData::AAAA(address), RecordType::AAAA) => {
                        addresses.push(IpAddr::V6(address.0));
                    }
                    _ => return Err(DnsResolveError::Lookup),
                }
            }
            Ok(())
        }
        Err(NetError::Dns(DnsError::NoRecordsFound(_))) => Ok(()),
        Err(_) => Err(DnsResolveError::Lookup),
    }
}

pub(super) struct SystemNat64PrefixDiscovery {
    _resolver: TokioResolver,
    lookup: Arc<dyn ExplicitLookup>,
}

impl SystemNat64PrefixDiscovery {
    pub(super) fn new() -> Result<Self, NetError> {
        let mut builder = TokioResolver::builder_tokio()?;
        let options = builder.options_mut();
        options.use_hosts_file = ResolveHosts::Never;
        options.cache_size = 0;
        options.attempts = 1;
        options.positive_min_ttl = None;
        options.positive_max_ttl = None;
        options.negative_min_ttl = None;
        options.negative_max_ttl = None;
        let resolver = builder.build()?;
        let lookup = Arc::new(resolver.clone());
        Ok(Self {
            _resolver: resolver,
            lookup,
        })
    }

    #[cfg(test)]
    fn with_lookup(resolver: TokioResolver, lookup: Arc<dyn ExplicitLookup>) -> Self {
        Self {
            _resolver: resolver,
            lookup,
        }
    }
}

#[async_trait]
impl Nat64PrefixDiscovery for SystemNat64PrefixDiscovery {
    async fn discover(&self, deadline: Instant) -> Result<Nat64Discovery, Nat64DiscoveryError> {
        if Instant::now() >= deadline {
            return Err(Nat64DiscoveryError::Deadline);
        }
        let query_deadline = min_deadline(
            deadline,
            Instant::now()
                .checked_add(DNS_TIMEOUT)
                .ok_or(Nat64DiscoveryError::Deadline)?,
        );
        let lookup = self.lookup.clone();
        let (a_result, aaaa_result) = timeout_at(query_deadline, async move {
            tokio::join!(
                lookup.lookup(IPV4ONLY_ARPA, RecordType::A),
                lookup.lookup(IPV4ONLY_ARPA, RecordType::AAAA)
            )
        })
        .await
        .map_err(|_| Nat64DiscoveryError::Deadline)?;

        parse_discovery_results(a_result, aaaa_result)
    }
}

fn parse_discovery_results(
    a_result: Result<Lookup, NetError>,
    aaaa_result: Result<Lookup, NetError>,
) -> Result<Nat64Discovery, Nat64DiscoveryError> {
    let received_std = StdInstant::now();
    let received_tokio = Instant::now();
    let a_lookup = a_result.map_err(|_| Nat64DiscoveryError::Lookup)?;
    validate_positive_deadline(a_lookup.valid_until(), received_std)?;
    let a_addresses = collect_ipv4_answers(&a_lookup)?;
    if !a_addresses.contains(&WKA_170) || !a_addresses.contains(&WKA_171) {
        return Err(Nat64DiscoveryError::MissingWellKnownAddresses);
    }

    match aaaa_result {
        Ok(lookup) => {
            validate_positive_deadline(lookup.valid_until(), received_std)?;
            let addresses = collect_ipv6_answers(&lookup)?;
            let prefixes = derive_nat64_prefixes(&addresses)?;
            let valid_std = a_lookup.valid_until().min(lookup.valid_until());
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::Present(prefixes),
                valid_until: convert_deadline(valid_std, received_std, received_tokio)?,
            })
        }
        Err(NetError::Dns(DnsError::NoRecordsFound(no_records)))
            if no_records.response_code == ResponseCode::NoError =>
        {
            let ttl = no_records
                .negative_ttl
                .filter(|ttl| *ttl > 0)
                .ok_or(Nat64DiscoveryError::InvalidNegativeResponse)?;
            let negative_deadline = received_std
                .checked_add(Duration::from_secs(u64::from(ttl)))
                .ok_or(Nat64DiscoveryError::InvalidTtl)?;
            let valid_std = a_lookup.valid_until().min(negative_deadline);
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: convert_deadline(valid_std, received_std, received_tokio)?,
            })
        }
        Err(NetError::Dns(DnsError::NoRecordsFound(_))) => {
            Err(Nat64DiscoveryError::InvalidNegativeResponse)
        }
        Err(_) => Err(Nat64DiscoveryError::Lookup),
    }
}

fn validate_positive_deadline(
    valid_until: StdInstant,
    now: StdInstant,
) -> Result<(), Nat64DiscoveryError> {
    if now >= valid_until {
        Err(Nat64DiscoveryError::InvalidTtl)
    } else {
        Ok(())
    }
}

fn convert_deadline(
    deadline: StdInstant,
    std_now: StdInstant,
    tokio_now: Instant,
) -> Result<Instant, Nat64DiscoveryError> {
    let remaining = deadline
        .checked_duration_since(std_now)
        .filter(|remaining| !remaining.is_zero())
        .ok_or(Nat64DiscoveryError::InvalidTtl)?;
    tokio_now
        .checked_add(remaining)
        .ok_or(Nat64DiscoveryError::InvalidTtl)
}

fn collect_ipv4_answers(lookup: &Lookup) -> Result<HashSet<Ipv4Addr>, Nat64DiscoveryError> {
    let mut addresses = HashSet::new();
    for record in lookup.answers() {
        if record.dns_class != DNSClass::IN {
            return Err(Nat64DiscoveryError::MalformedAnswer);
        }
        match &record.data {
            RData::A(address) => {
                addresses.insert(address.0);
            }
            _ => return Err(Nat64DiscoveryError::MalformedAnswer),
        }
    }
    if addresses.is_empty() {
        return Err(Nat64DiscoveryError::MalformedAnswer);
    }
    Ok(addresses)
}

fn collect_ipv6_answers(lookup: &Lookup) -> Result<Vec<Ipv6Addr>, Nat64DiscoveryError> {
    let mut addresses = Vec::new();
    for record in lookup.answers() {
        if record.dns_class != DNSClass::IN {
            return Err(Nat64DiscoveryError::MalformedAnswer);
        }
        match &record.data {
            RData::AAAA(address) => addresses.push(address.0),
            _ => return Err(Nat64DiscoveryError::MalformedAnswer),
        }
    }
    if addresses.is_empty() {
        return Err(Nat64DiscoveryError::MalformedAnswer);
    }
    Ok(addresses)
}

fn derive_nat64_prefixes(addresses: &[Ipv6Addr]) -> Result<Vec<Ipv6Net>, Nat64DiscoveryError> {
    let mut candidates: BTreeMap<Ipv6Net, BTreeSet<Ipv4Addr>> = BTreeMap::new();
    let mut matches_per_address = vec![Vec::new(); addresses.len()];

    for (index, address) in addresses.iter().copied().enumerate() {
        for prefix_len in SUPPORTED_PREFIX_LENGTHS {
            if prefix_len <= 64 && u_octet(address) != 0 {
                continue;
            }
            let embedded = extract_rfc6052(address, prefix_len);
            if embedded != WKA_170 && embedded != WKA_171 {
                continue;
            }
            let candidate = Ipv6Net::new(address, prefix_len)
                .map_err(|_| Nat64DiscoveryError::MalformedAnswer)?
                .trunc();
            candidates.entry(candidate).or_default().insert(embedded);
            matches_per_address[index].push(candidate);
        }
    }

    let complete: BTreeSet<Ipv6Net> = candidates
        .into_iter()
        .filter_map(|(prefix, addresses)| {
            (addresses.contains(&WKA_170) && addresses.contains(&WKA_171)).then_some(prefix)
        })
        .collect();
    if complete.is_empty() {
        return Err(Nat64DiscoveryError::MalformedAnswer);
    }
    for matches in matches_per_address {
        let retained = matches
            .into_iter()
            .filter(|candidate| complete.contains(candidate))
            .collect::<BTreeSet<_>>();
        if retained.len() != 1 {
            return Err(Nat64DiscoveryError::AmbiguousAnswer);
        }
    }

    let prefixes = complete.into_iter().collect::<Vec<_>>();
    AddressPolicy::with_nat64_prefixes(prefixes.iter().copied())?;
    Ok(prefixes)
}

fn extract_rfc6052(address: Ipv6Addr, prefix_len: u8) -> Ipv4Addr {
    let bits = u128::from(address);
    let embedded = match prefix_len {
        32 => (bits >> 64) as u32,
        40 => ((((bits >> 64) & 0x00ff_ffff) << 8) | ((bits >> 48) & 0xff)) as u32,
        48 => ((((bits >> 64) & 0xffff) << 16) | ((bits >> 40) & 0xffff)) as u32,
        56 => ((((bits >> 64) & 0xff) << 24) | ((bits >> 32) & 0x00ff_ffff)) as u32,
        64 => ((bits >> 24) & u128::from(u32::MAX)) as u32,
        96 => bits as u32,
        _ => unreachable!("RFC 6052 prefix length is fixed"),
    };
    Ipv4Addr::from(embedded)
}

fn u_octet(address: Ipv6Addr) -> u8 {
    ((u128::from(address) >> 56) & 0xff) as u8
}

fn absolute_name(host: &str) -> String {
    if host.ends_with('.') {
        host.to_owned()
    } else {
        format!("{host}.")
    }
}

fn min_deadline(left: Instant, right: Instant) -> Instant {
    if left <= right { left } else { right }
}

#[derive(Clone)]
pub(super) struct Nat64Snapshot {
    pub(super) generation: u64,
    pub(super) valid_until: Option<Instant>,
    pub(super) address_policy: AddressPolicy,
}

impl Nat64Snapshot {
    pub(super) fn is_expired(&self, now: Instant) -> bool {
        self.valid_until.is_some_and(|deadline| now >= deadline)
    }

    pub(super) fn same_version(&self, other: &Self) -> bool {
        self.generation == other.generation && self.valid_until == other.valid_until
    }
}

enum Nat64SnapshotMode {
    Fixed(Arc<Nat64Snapshot>),
    Automatic {
        discovery: Arc<dyn Nat64PrefixDiscovery>,
        snapshot: RwLock<Option<Arc<Nat64Snapshot>>>,
        refresh: Mutex<()>,
    },
}

pub(super) struct Nat64Snapshots {
    mode: Nat64SnapshotMode,
}

impl Nat64Snapshots {
    pub(super) fn disabled() -> Self {
        Self {
            mode: Nat64SnapshotMode::Fixed(Arc::new(Nat64Snapshot {
                generation: 0,
                valid_until: None,
                address_policy: AddressPolicy::public_only(),
            })),
        }
    }

    pub(super) fn static_prefixes(prefixes: Vec<Ipv6Net>) -> Result<Self, AddressPolicyError> {
        Ok(Self {
            mode: Nat64SnapshotMode::Fixed(Arc::new(Nat64Snapshot {
                generation: 0,
                valid_until: None,
                address_policy: AddressPolicy::with_nat64_prefixes(prefixes)?,
            })),
        })
    }

    pub(super) fn automatic(discovery: Arc<dyn Nat64PrefixDiscovery>) -> Self {
        Self {
            mode: Nat64SnapshotMode::Automatic {
                discovery,
                snapshot: RwLock::new(None),
                refresh: Mutex::new(()),
            },
        }
    }

    pub(super) async fn current(
        &self,
        total_deadline: Instant,
    ) -> Result<Arc<Nat64Snapshot>, Nat64DiscoveryError> {
        match &self.mode {
            Nat64SnapshotMode::Fixed(snapshot) => Ok(snapshot.clone()),
            Nat64SnapshotMode::Automatic {
                discovery,
                snapshot,
                refresh,
            } => {
                let now = Instant::now();
                if now >= total_deadline {
                    return Err(Nat64DiscoveryError::Deadline);
                }
                if let Some(current) = snapshot.read().await.clone()
                    && !current.is_expired(now)
                {
                    return Ok(current);
                }

                let _guard = timeout_at(total_deadline, refresh.lock())
                    .await
                    .map_err(|_| Nat64DiscoveryError::Deadline)?;
                let now = Instant::now();
                if let Some(current) = snapshot.read().await.clone()
                    && !current.is_expired(now)
                {
                    return Ok(current);
                }
                let discovery_deadline = min_deadline(
                    total_deadline,
                    now.checked_add(DNS_TIMEOUT)
                        .ok_or(Nat64DiscoveryError::Deadline)?,
                );
                let discovered = discovery.discover(discovery_deadline).await?;
                if Instant::now() >= discovered.valid_until {
                    return Err(Nat64DiscoveryError::InvalidTtl);
                }
                let policy = match discovered.state {
                    Nat64DiscoveryState::Present(prefixes) => {
                        AddressPolicy::with_nat64_prefixes(prefixes)?
                    }
                    Nat64DiscoveryState::NotPresent => AddressPolicy::public_only(),
                };
                let previous = snapshot.read().await.clone();
                let generation = previous.as_ref().map_or(1, |previous| {
                    if previous.address_policy == policy {
                        previous.generation
                    } else {
                        previous.generation.saturating_add(1)
                    }
                });
                let next = Arc::new(Nat64Snapshot {
                    generation,
                    valid_until: Some(discovered.valid_until),
                    address_policy: policy,
                });
                *snapshot.write().await = Some(next.clone());
                Ok(next)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex as StdMutex},
        time::{Duration, Instant as StdInstant},
    };

    use hickory_resolver::{
        TokioResolver,
        config::ResolveHosts,
        lookup::Lookup,
        net::{NetError, NoRecords},
        proto::{
            op::{Query, ResponseCode},
            rr::{
                Name, RData, Record, RecordType,
                rdata::{A, AAAA},
            },
        },
    };
    use ipnet::Ipv6Net;
    use tokio::time::Instant;

    use super::{
        ExplicitLookup, Nat64Discovery, Nat64DiscoveryError, Nat64DiscoveryState,
        Nat64PrefixDiscovery, Nat64Snapshots, SystemNat64PrefixDiscovery, derive_nat64_prefixes,
        parse_discovery_results,
    };
    use crate::feeds::test_support::FakeNat64Discovery;

    #[tokio::test]
    async fn nat64_discovery_bypasses_hosts_and_response_cache() {
        let discovery = SystemNat64PrefixDiscovery::new().expect("system resolver configuration");
        let options = discovery._resolver.options();

        assert_eq!(options.use_hosts_file, ResolveHosts::Never);
        assert_eq!(options.cache_size, 0);
        assert_eq!(options.attempts, 1);
        assert_eq!(options.positive_min_ttl, None);
        assert_eq!(options.positive_max_ttl, None);
        assert_eq!(options.negative_min_ttl, None);
        assert_eq!(options.negative_max_ttl, None);

        let lookup = Arc::new(ScriptedLookup::new(vec![
            ScriptedLookupResult::Delayed(
                Duration::ZERO,
                Ok(lookup_v4([super::WKA_170, super::WKA_171], 60)),
            ),
            ScriptedLookupResult::Delayed(
                Duration::ZERO,
                Err(no_records(
                    RecordType::AAAA,
                    ResponseCode::NoError,
                    Some(60),
                )),
            ),
            ScriptedLookupResult::Delayed(
                Duration::ZERO,
                Ok(lookup_v4([super::WKA_170, super::WKA_171], 60)),
            ),
            ScriptedLookupResult::Delayed(
                Duration::ZERO,
                Err(no_records(
                    RecordType::AAAA,
                    ResponseCode::NoError,
                    Some(60),
                )),
            ),
        ]));
        let resolver = TokioResolver::builder_tokio().unwrap().build().unwrap();
        let discovery = SystemNat64PrefixDiscovery::with_lookup(resolver, lookup.clone());
        discovery
            .discover(Instant::now() + Duration::from_secs(3))
            .await
            .unwrap();
        discovery
            .discover(Instant::now() + Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(lookup.calls(), 4);
    }

    #[test]
    fn nat64_absence_requires_verified_a_and_nodata_aaaa() {
        let a = lookup_v4([super::WKA_170, super::WKA_171], 60);
        let nodata = no_records(RecordType::AAAA, ResponseCode::NoError, Some(30));
        let result = parse_discovery_results(Ok(a), Err(nodata)).expect("verified absence");
        assert_eq!(result.state, Nat64DiscoveryState::NotPresent);

        let missing_wka = lookup_v4([super::WKA_170], 60);
        assert_eq!(
            parse_discovery_results(
                Ok(missing_wka),
                Err(no_records(
                    RecordType::AAAA,
                    ResponseCode::NoError,
                    Some(30)
                ))
            )
            .unwrap_err(),
            Nat64DiscoveryError::MissingWellKnownAddresses
        );
        let a = lookup_v4([super::WKA_170, super::WKA_171], 60);
        assert_eq!(
            parse_discovery_results(
                Ok(a),
                Err(no_records(
                    RecordType::AAAA,
                    ResponseCode::NXDomain,
                    Some(30)
                ))
            )
            .unwrap_err(),
            Nat64DiscoveryError::InvalidNegativeResponse
        );
    }

    #[test]
    fn nat64_discovery_covers_all_six_prefix_lengths_and_multiple_prefixes() {
        let prefixes = [
            "2001:300::/32",
            "2001:400:100::/40",
            "2001:500:200::/48",
            "2001:600:300:400::/56",
            "2001:700:400:500::/64",
            "2001:800:500:600::/96",
        ]
        .map(|raw| raw.parse::<Ipv6Net>().unwrap());
        let addresses = prefixes
            .iter()
            .flat_map(|prefix| {
                [
                    synthesize(*prefix, super::WKA_170),
                    synthesize(*prefix, super::WKA_171),
                ]
            })
            .collect::<Vec<_>>();
        assert_eq!(derive_nat64_prefixes(&addresses).unwrap(), prefixes);
    }

    #[test]
    fn nat64_discovery_rejects_ambiguous_wka_mappings_and_negative_responses() {
        let ambiguous = ["2001:c000:aa::", "2001:c000:ab::"]
            .map(|raw| raw.parse::<std::net::Ipv6Addr>().unwrap());
        assert!(matches!(
            derive_nat64_prefixes(&ambiguous),
            Err(Nat64DiscoveryError::AmbiguousAnswer | Nat64DiscoveryError::MalformedAnswer)
        ));

        let a = lookup_v4([super::WKA_170, super::WKA_171], 60);
        assert_eq!(
            parse_discovery_results(
                Ok(a),
                Err(no_records(RecordType::AAAA, ResponseCode::NoError, None))
            )
            .unwrap_err(),
            Nat64DiscoveryError::InvalidNegativeResponse
        );
    }

    #[tokio::test(start_paused = true)]
    async fn nat64_a_and_aaaa_share_one_three_second_deadline() {
        let lookup = Arc::new(ScriptedLookup::new(vec![
            ScriptedLookupResult::Delayed(
                Duration::from_secs(4),
                Ok(lookup_v4([super::WKA_170, super::WKA_171], 60)),
            ),
            ScriptedLookupResult::Delayed(Duration::from_secs(4), Ok(lookup_v6([], 60))),
        ]));
        let resolver = TokioResolver::builder_tokio().unwrap().build().unwrap();
        let discovery = SystemNat64PrefixDiscovery::with_lookup(resolver, lookup);
        let deadline = Instant::now() + Duration::from_secs(30);
        assert_eq!(
            discovery.discover(deadline).await.unwrap_err(),
            Nat64DiscoveryError::Deadline
        );
        assert_eq!(Instant::now(), deadline - Duration::from_secs(27));
    }

    #[tokio::test(start_paused = true)]
    async fn nat64_snapshot_publication_renews_deadlines_and_versions_policy_changes() {
        let started = Instant::now();
        let prefix = "2001:300::/96".parse::<Ipv6Net>().unwrap();
        let discovery = Arc::new(FakeNat64Discovery::new(vec![
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: started + Duration::from_secs(1),
            }),
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::NotPresent,
                valid_until: started + Duration::from_secs(2),
            }),
            Ok(Nat64Discovery {
                state: Nat64DiscoveryState::Present(vec![prefix]),
                valid_until: started + Duration::from_secs(3),
            }),
        ]));
        let snapshots = Nat64Snapshots::automatic(discovery);
        let total_deadline = started + Duration::from_secs(30);

        let first = snapshots.current(total_deadline).await.unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.valid_until, Some(started + Duration::from_secs(1)));

        tokio::time::advance(Duration::from_secs(1)).await;
        let renewed = snapshots.current(total_deadline).await.unwrap();
        assert_eq!(renewed.generation, first.generation);
        assert_eq!(renewed.valid_until, Some(started + Duration::from_secs(2)));
        assert!(!first.same_version(&renewed));

        tokio::time::advance(Duration::from_secs(1)).await;
        let changed = snapshots.current(total_deadline).await.unwrap();
        assert_eq!(changed.generation, renewed.generation + 1);
        assert_eq!(changed.valid_until, Some(started + Duration::from_secs(3)));
        assert_ne!(changed.address_policy, renewed.address_policy);
    }

    enum ScriptedLookupResult {
        Delayed(Duration, Result<Lookup, NetError>),
    }

    struct ScriptedLookup {
        results: StdMutex<VecDeque<ScriptedLookupResult>>,
        calls: StdMutex<usize>,
    }

    impl ScriptedLookup {
        fn new(results: Vec<ScriptedLookupResult>) -> Self {
            Self {
                results: StdMutex::new(results.into()),
                calls: StdMutex::new(0),
            }
        }

        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl ExplicitLookup for ScriptedLookup {
        async fn lookup(&self, _name: &str, _record_type: RecordType) -> Result<Lookup, NetError> {
            *self.calls.lock().unwrap() += 1;
            let ScriptedLookupResult::Delayed(delay, result) =
                self.results.lock().unwrap().pop_front().unwrap();
            tokio::time::sleep(delay).await;
            result
        }
    }

    fn lookup_v4<const N: usize>(addresses: [std::net::Ipv4Addr; N], ttl: u32) -> Lookup {
        let name = Name::from_ascii("ipv4only.arpa.").unwrap();
        let query = Query::query(name.clone(), RecordType::A);
        let records =
            addresses.map(|address| Record::from_rdata(name.clone(), ttl, RData::A(A(address))));
        Lookup::new_with_deadline(
            query,
            records,
            StdInstant::now() + Duration::from_secs(u64::from(ttl)),
        )
    }

    fn lookup_v6<const N: usize>(addresses: [std::net::Ipv6Addr; N], ttl: u32) -> Lookup {
        let name = Name::from_ascii("ipv4only.arpa.").unwrap();
        let query = Query::query(name.clone(), RecordType::AAAA);
        let records = addresses
            .map(|address| Record::from_rdata(name.clone(), ttl, RData::AAAA(AAAA(address))));
        Lookup::new_with_deadline(
            query,
            records,
            StdInstant::now() + Duration::from_secs(u64::from(ttl)),
        )
    }

    fn no_records(
        record_type: RecordType,
        response_code: ResponseCode,
        ttl: Option<u32>,
    ) -> NetError {
        let query = Query::query(Name::from_ascii("ipv4only.arpa.").unwrap(), record_type);
        let mut no_records = NoRecords::new(query, response_code);
        no_records.negative_ttl = ttl;
        no_records.into()
    }

    fn synthesize(prefix: Ipv6Net, ipv4: std::net::Ipv4Addr) -> std::net::Ipv6Addr {
        let mut bits = u128::from(prefix.network());
        let value = u128::from(u32::from(ipv4));
        bits |= match prefix.prefix_len() {
            32 => value << 64,
            40 => ((value >> 8) << 64) | ((value & 0xff) << 48),
            48 => ((value >> 16) << 64) | ((value & 0xffff) << 40),
            56 => ((value >> 24) << 64) | ((value & 0x00ff_ffff) << 32),
            64 => value << 24,
            96 => value,
            _ => unreachable!(),
        };
        std::net::Ipv6Addr::from(bits)
    }
}
