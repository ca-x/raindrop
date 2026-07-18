use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::Ipv6Net;

use super::{AddressDecision, AddressDenyReason, AddressPolicyError};

const SUPPORTED_PREFIX_LENGTHS: [u8; 6] = [32, 40, 48, 56, 64, 96];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressPolicy {
    nat64_prefixes: Vec<Ipv6Net>,
}

impl AddressPolicy {
    #[must_use]
    pub const fn public_only() -> Self {
        Self {
            nat64_prefixes: Vec::new(),
        }
    }

    pub fn with_nat64_prefixes(
        prefixes: impl IntoIterator<Item = Ipv6Net>,
    ) -> Result<Self, AddressPolicyError> {
        let mut validated = Vec::new();
        for prefix in prefixes {
            if !SUPPORTED_PREFIX_LENGTHS.contains(&prefix.prefix_len()) {
                return Err(AddressPolicyError::InvalidPrefixLength);
            }
            if prefix.addr() != prefix.network() {
                return Err(AddressPolicyError::NonCanonical);
            }
            if overlaps_special_range(&prefix) {
                return Err(AddressPolicyError::SpecialRange);
            }
            if prefix.prefix_len() == 96 && u_octet(prefix.addr()) != 0 {
                return Err(AddressPolicyError::NonZeroUOctet);
            }
            if !native_ipv6_allowed(prefix.network()) || !native_ipv6_allowed(last_address(prefix))
            {
                return Err(AddressPolicyError::OutsideAllowedIpv6);
            }
            if validated.iter().any(|other: &Ipv6Net| {
                other.contains(&prefix.network()) || prefix.contains(&other.network())
            }) {
                return Err(AddressPolicyError::Overlap);
            }
            validated.push(prefix);
        }

        Ok(Self {
            nat64_prefixes: validated,
        })
    }

    #[must_use]
    pub fn classify(&self, address: IpAddr) -> AddressDecision {
        match address {
            IpAddr::V4(address) => classify_native_ipv4(address),
            IpAddr::V6(address) => self.classify_ipv6(address),
        }
    }

    fn classify_ipv6(&self, address: Ipv6Addr) -> AddressDecision {
        if let Some(embedded) = address.to_ipv4_mapped() {
            return classify_embedded_ipv4(embedded);
        }

        let bits = u128::from(address);
        if bits >> 32 == 0 {
            return classify_embedded_ipv4(Ipv4Addr::from(bits as u32));
        }

        if in_ipv6_prefix(address, Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96) {
            return classify_embedded_ipv4(Ipv4Addr::from(bits as u32));
        }

        for prefix in &self.nat64_prefixes {
            if prefix.contains(&address) {
                if prefix.prefix_len() <= 64 && u_octet(address) != 0 {
                    return AddressDecision::Denied(AddressDenyReason::Nat64UOctet);
                }
                return classify_embedded_ipv4(extract_rfc6052(address, prefix.prefix_len()));
            }
        }

        if in_ipv6_prefix(address, Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16) {
            return classify_embedded_ipv4(Ipv4Addr::from(
                ((bits >> 80) & u32::MAX as u128) as u32,
            ));
        }

        if in_ipv6_prefix(address, Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32) {
            let server = Ipv4Addr::from(((bits >> 64) & u32::MAX as u128) as u32);
            if classify_native_ipv4(server) != AddressDecision::Allowed {
                return AddressDecision::Denied(AddressDenyReason::TeredoServer);
            }
            let client = Ipv4Addr::from(!(bits as u32));
            if classify_native_ipv4(client) != AddressDecision::Allowed {
                return AddressDecision::Denied(AddressDenyReason::TeredoClient);
            }
            return AddressDecision::Allowed;
        }

        if in_ipv6_prefix(address, Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48) {
            return AddressDecision::Denied(AddressDenyReason::LocalUseNat64);
        }

        if native_ipv6_allowed(address) {
            AddressDecision::Allowed
        } else if !in_ipv6_prefix(address, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3) {
            AddressDecision::Denied(AddressDenyReason::Ipv6OutsideGlobalUnicast)
        } else {
            AddressDecision::Denied(AddressDenyReason::Ipv6Special)
        }
    }
}

fn classify_native_ipv4(address: Ipv4Addr) -> AddressDecision {
    let denied = [
        (Ipv4Addr::new(0, 0, 0, 0), 8),
        (Ipv4Addr::new(10, 0, 0, 0), 8),
        (Ipv4Addr::new(100, 64, 0, 0), 10),
        (Ipv4Addr::new(127, 0, 0, 0), 8),
        (Ipv4Addr::new(169, 254, 0, 0), 16),
        (Ipv4Addr::new(172, 16, 0, 0), 12),
        (Ipv4Addr::new(192, 0, 0, 0), 24),
        (Ipv4Addr::new(192, 0, 2, 0), 24),
        (Ipv4Addr::new(192, 88, 99, 0), 24),
        (Ipv4Addr::new(192, 168, 0, 0), 16),
        (Ipv4Addr::new(198, 18, 0, 0), 15),
        (Ipv4Addr::new(198, 51, 100, 0), 24),
        (Ipv4Addr::new(203, 0, 113, 0), 24),
        (Ipv4Addr::new(224, 0, 0, 0), 4),
        (Ipv4Addr::new(240, 0, 0, 0), 4),
    ];

    if denied
        .iter()
        .any(|&(network, length)| in_ipv4_prefix(address, network, length))
    {
        AddressDecision::Denied(AddressDenyReason::Ipv4Special)
    } else {
        AddressDecision::Allowed
    }
}

fn classify_embedded_ipv4(address: Ipv4Addr) -> AddressDecision {
    match classify_native_ipv4(address) {
        AddressDecision::Allowed => AddressDecision::Allowed,
        AddressDecision::Denied(_) => AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
    }
}

fn native_ipv6_allowed(address: Ipv6Addr) -> bool {
    in_ipv6_prefix(address, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        && !in_ipv6_prefix(address, Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23)
        && !in_ipv6_prefix(address, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32)
        && !in_ipv6_prefix(address, Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20)
}

fn overlaps_special_range(prefix: &Ipv6Net) -> bool {
    let ranges = [
        net(Ipv6Addr::UNSPECIFIED, 96),
        net(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0), 96),
        net(Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96),
        net(Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48),
        net(Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
        net(Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32),
        net(Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
        net(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
        net(Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
    ];

    ranges
        .iter()
        .any(|range| range.contains(&prefix.network()) || prefix.contains(&range.network()))
}

pub(super) fn extract_rfc6052(address: Ipv6Addr, prefix_len: u8) -> Ipv4Addr {
    let bits = u128::from(address);
    let embedded = match prefix_len {
        32 => (bits >> 64) as u32,
        40 => ((((bits >> 64) & 0x00ff_ffff) << 8) | ((bits >> 48) & 0xff)) as u32,
        48 => ((((bits >> 64) & 0xffff) << 16) | ((bits >> 40) & 0xffff)) as u32,
        56 => ((((bits >> 64) & 0xff) << 24) | ((bits >> 32) & 0x00ff_ffff)) as u32,
        64 => ((bits >> 24) & u32::MAX as u128) as u32,
        96 => bits as u32,
        _ => unreachable!("prefix lengths are validated at construction"),
    };
    Ipv4Addr::from(embedded)
}

fn u_octet(address: Ipv6Addr) -> u8 {
    ((u128::from(address) >> 56) & 0xff) as u8
}

fn in_ipv4_prefix(address: Ipv4Addr, network: Ipv4Addr, prefix_len: u8) -> bool {
    let mask = u32::MAX
        .checked_shl(u32::from(32 - prefix_len))
        .unwrap_or(0);
    u32::from(address) & mask == u32::from(network) & mask
}

fn in_ipv6_prefix(address: Ipv6Addr, network: Ipv6Addr, prefix_len: u8) -> bool {
    let mask = u128::MAX
        .checked_shl(u32::from(128 - prefix_len))
        .unwrap_or(0);
    u128::from(address) & mask == u128::from(network) & mask
}

fn last_address(prefix: Ipv6Net) -> Ipv6Addr {
    let host_bits = 128 - u32::from(prefix.prefix_len());
    let host_mask = u128::MAX.checked_shr(128 - host_bits).unwrap_or(0);
    Ipv6Addr::from(u128::from(prefix.network()) | host_mask)
}

fn net(address: Ipv6Addr, prefix_len: u8) -> Ipv6Net {
    Ipv6Net::new(address, prefix_len).expect("fixed IPv6 network is valid")
}
