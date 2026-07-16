use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::rc::Rc;
use std::str::FromStr;

use base64::Engine;
use http::HeaderValue;
use ipnet::Ipv6Net;
use raindrop::feeds::{
    AddressDecision, AddressDenyReason, AddressPolicy, AddressPolicyError, EntryIdentity,
    FeedUrlError, FeedUrlPolicy, IdentityError, IdentityKind, JitterSource, OpaqueValidator,
    RefreshResult, RefreshSchedule, RetryAfter, RetryAfterError, ScheduleError, StableEntryFields,
    ValidatorError, ValidatorSet,
};
use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time};

#[test]
fn feed_url_policy_normalizes_https_without_exposing_the_complete_url() {
    let policy = FeedUrlPolicy::new(false);
    let normalized = policy
        .normalize("HTTPS://B\u{dc}CHER.Example.:443?x=1&x=2#secret")
        .unwrap();

    assert_eq!(normalized.scheme(), "https");
    assert_eq!(normalized.canonical_host(), "xn--bcher-kva.example");
    assert_eq!(normalized.effective_port(), 443);

    let debug = format!("{normalized:?}");
    assert!(!debug.contains("x=1"));
    assert!(!debug.contains("secret"));
}

#[test]
fn feed_url_policy_rejects_insecure_http_by_default() {
    let error = FeedUrlPolicy::new(false)
        .normalize("http://example.com/feed")
        .unwrap_err();

    assert_eq!(error, FeedUrlError::InsecureHttpDisabled);
}

#[test]
fn feed_url_policy_preserves_query_order_and_removes_only_non_identity_parts() {
    let policy = FeedUrlPolicy::new(false);
    let normalized = policy
        .normalize("https://EXAMPLE.com.:443?x=1&x=2#fragment")
        .unwrap();
    let equivalent = policy.normalize("https://example.com/?x=1&x=2").unwrap();
    let reordered = policy.normalize("https://example.com/?x=2&x=1").unwrap();

    assert_eq!(normalized, equivalent);
    assert_ne!(normalized, reordered);
    assert_eq!(normalized.url_hash().len(), 64);
    assert!(
        normalized
            .url_hash()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn feed_url_policy_rejects_unsafe_or_non_absolute_inputs_with_typed_errors() {
    let policy = FeedUrlPolicy::new(false);
    let cases = [
        ("", FeedUrlError::Empty),
        ("https://@example.com/", FeedUrlError::CredentialsForbidden),
        (
            "https://user:password@example.com/",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            "https://user:password@example.com:bad/",
            FeedUrlError::CredentialsForbidden,
        ),
        ("https://example.com/a path", FeedUrlError::ControlCharacter),
        ("\thttps://example.com/", FeedUrlError::ControlCharacter),
        ("https://example.com/\u{7f}", FeedUrlError::ControlCharacter),
        ("https://example.com/\u{85}", FeedUrlError::ControlCharacter),
        ("/relative", FeedUrlError::Invalid),
        ("//example.com/feed", FeedUrlError::Invalid),
        ("ftp://example.com/feed", FeedUrlError::UnsupportedScheme),
        ("https://example.com:bad/", FeedUrlError::Invalid),
        ("https://example.com:/", FeedUrlError::Invalid),
        ("https://bad_host.example/", FeedUrlError::Invalid),
        ("https://-bad.example/", FeedUrlError::Invalid),
        ("https://example.com../", FeedUrlError::Invalid),
    ];

    for (raw, expected) in cases {
        let error = policy.normalize(raw).unwrap_err();
        assert_eq!(error, expected, "raw input category did not match");
        let rendered = format!("{error:?}: {error}");
        if !raw.is_empty() {
            assert!(!rendered.contains(raw), "typed error leaked raw URL");
        }
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn feed_url_authority_scanning_does_not_treat_path_or_query_at_signs_as_userinfo() {
    let policy = FeedUrlPolicy::new(false);
    for raw in [
        "https://example.com/path@publisher/feed",
        "https://example.com/feed?contact=editor@example.net",
        "https://example.com\\@publisher/feed",
    ] {
        let normalized = policy.normalize(raw).unwrap();
        assert_eq!(normalized.canonical_host(), "example.com");
    }
}

#[test]
fn feed_url_policy_requires_literal_double_slash_after_http_schemes() {
    let policy = FeedUrlPolicy::new(true);
    for (raw, expected) in [
        ("https:/example.com/feed", FeedUrlError::Invalid),
        (r"https:\example.com/feed", FeedUrlError::Invalid),
        (
            "https:/user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            r"https:\user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        ("https:///example.com/feed", FeedUrlError::Invalid),
        ("https:////example.com/feed", FeedUrlError::Invalid),
        (r"https://\example.com/feed", FeedUrlError::Invalid),
        (
            "https:///user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            "https:////user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            "https:///@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            "https:////@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        (
            r"https://\user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
        ("https://", FeedUrlError::Invalid),
        ("https://?token=secret", FeedUrlError::Invalid),
        ("https://#fragment", FeedUrlError::Invalid),
        ("https://example.com:/feed", FeedUrlError::Invalid),
        ("HTTP:/example.com/feed", FeedUrlError::Invalid),
        ("HTTP:///example.com/feed", FeedUrlError::Invalid),
        (
            r"HtTp:\user@example.com/feed",
            FeedUrlError::CredentialsForbidden,
        ),
    ] {
        assert_eq!(
            policy.normalize(raw),
            Err(expected),
            "malformed HTTP authority was accepted: {raw}"
        );
    }
}

#[test]
fn feed_url_policy_enforces_raw_and_normalized_size_limits() {
    let policy = FeedUrlPolicy::new(false);
    let exact_limit = format!("https://example.com/?{}", "a".repeat(4_075));
    assert_eq!(exact_limit.len(), 4_096);
    assert!(policy.normalize(&exact_limit).is_ok());

    let raw_oversize = format!("https://example.com/{}", "a".repeat(4_077));
    assert_eq!(raw_oversize.len(), 4_097);
    assert_eq!(policy.normalize(&raw_oversize), Err(FeedUrlError::TooLong));

    let normalized_oversize = format!("https://example.com/?{}", "\u{e9}".repeat(2_000));
    assert!(normalized_oversize.len() <= 4_096);
    assert_eq!(
        policy.normalize(&normalized_oversize),
        Err(FeedUrlError::TooLong)
    );
}

#[test]
fn feed_url_policy_enforces_strict_dns_label_and_host_boundaries() {
    let policy = FeedUrlPolicy::new(false);
    let maximum_host = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(maximum_host.len(), 253);
    assert!(
        policy
            .normalize(&format!("https://{maximum_host}/feed"))
            .is_ok()
    );

    let oversized_host = format!("{maximum_host}.e");
    assert!(oversized_host.len() > 253);
    assert_eq!(
        policy.normalize(&format!("https://{oversized_host}/feed")),
        Err(FeedUrlError::Invalid)
    );
    assert_eq!(
        policy.normalize(&format!("https://{}.example/feed", "a".repeat(64))),
        Err(FeedUrlError::Invalid)
    );
}

#[test]
fn feed_url_policy_accepts_http_only_when_explicit_and_forbids_downgrades() {
    assert_eq!(
        FeedUrlPolicy::new(false).normalize("http://example.com/feed"),
        Err(FeedUrlError::InsecureHttpDisabled)
    );

    let policy = FeedUrlPolicy::new(true);
    let http = policy.normalize("http://example.com:80").unwrap();
    assert_eq!(http.scheme(), "http");
    assert_eq!(http.effective_port(), 80);

    let https = policy.normalize("https://example.com/feed").unwrap();
    assert_eq!(
        policy.normalize_redirect(&https, "http://example.com/feed"),
        Err(FeedUrlError::HttpsDowngrade)
    );
}

#[test]
fn feed_url_policy_canonicalizes_nonstandard_ipv4_before_address_checks() {
    let policy = FeedUrlPolicy::new(false);
    let address_policy = AddressPolicy::public_only();
    for raw in [
        "https://127.1/feed",
        "https://0x7f000001/feed",
        "https://2130706433/feed",
    ] {
        let normalized = policy.normalize(raw).unwrap();
        assert_eq!(normalized.canonical_host(), "127.0.0.1");
        assert_eq!(
            address_policy.classify(IpAddr::from_str(normalized.canonical_host()).unwrap()),
            AddressDecision::Denied(AddressDenyReason::Ipv4Special)
        );
    }
}

#[test]
fn normalized_feed_url_debug_redacts_sensitive_components() {
    let raw = "https://example.com/private/path?token=top-secret#publisher-fragment";
    let normalized = FeedUrlPolicy::new(false).normalize(raw).unwrap();
    let debug = format!("{normalized:?}");

    for secret in ["private", "token", "top-secret", "publisher-fragment"] {
        assert!(!debug.contains(secret));
    }
    assert!(debug.contains("example.com"));
}

#[test]
fn address_policy_denies_every_frozen_ipv4_cidr_boundary() {
    let policy = AddressPolicy::public_only();
    let denied_boundaries = [
        ("0.0.0.0", "0.255.255.255"),
        ("10.0.0.0", "10.255.255.255"),
        ("100.64.0.0", "100.127.255.255"),
        ("127.0.0.0", "127.255.255.255"),
        ("169.254.0.0", "169.254.255.255"),
        ("172.16.0.0", "172.31.255.255"),
        ("192.0.0.0", "192.0.0.255"),
        ("192.0.2.0", "192.0.2.255"),
        ("192.88.99.0", "192.88.99.255"),
        ("192.168.0.0", "192.168.255.255"),
        ("198.18.0.0", "198.19.255.255"),
        ("198.51.100.0", "198.51.100.255"),
        ("203.0.113.0", "203.0.113.255"),
        ("224.0.0.0", "239.255.255.255"),
        ("240.0.0.0", "255.255.255.255"),
    ];

    for (first, last) in denied_boundaries {
        for address in [first, last] {
            assert_eq!(
                policy.classify(IpAddr::V4(Ipv4Addr::from_str(address).unwrap())),
                AddressDecision::Denied(AddressDenyReason::Ipv4Special),
                "unexpected decision at {address}"
            );
        }
    }

    for address in ["1.1.1.1", "8.8.8.8", "192.0.1.255", "223.255.255.255"] {
        assert_eq!(
            policy.classify(IpAddr::V4(Ipv4Addr::from_str(address).unwrap())),
            AddressDecision::Allowed,
            "public complement address was denied: {address}"
        );
    }
}

#[test]
fn address_policy_applies_frozen_native_ipv6_boundaries() {
    let policy = AddressPolicy::public_only();
    let denied = [
        "1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "4000::",
        "2001::",
        "2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff",
        "2001:db8::",
        "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff",
        "3fff::",
        "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff",
    ];
    for address in denied {
        assert!(matches!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(address).unwrap())),
            AddressDecision::Denied(_)
        ));
    }

    for address in ["2001:200::", "2001:4860:4860::8888", "2606:4700:4700::1111"] {
        assert_eq!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(address).unwrap())),
            AddressDecision::Allowed
        );
    }
}

#[test]
fn address_policy_unwraps_all_reviewed_ipv4_transition_forms_first() {
    let policy = AddressPolicy::public_only();
    let cases = [
        ("::ffff:8.8.8.8", AddressDecision::Allowed),
        (
            "::ffff:127.0.0.1",
            AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
        ),
        ("::8.8.8.8", AddressDecision::Allowed),
        (
            "::10.0.0.1",
            AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
        ),
        ("64:ff9b::808:808", AddressDecision::Allowed),
        (
            "64:ff9b::a00:1",
            AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
        ),
        ("2002:0808:0808::", AddressDecision::Allowed),
        (
            "2002:0a00:0001::",
            AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
        ),
        ("2001:0:808:808:0:0:fefe:fefe", AddressDecision::Allowed),
        (
            "2001:0:a00:1:0:0:fefe:fefe",
            AddressDecision::Denied(AddressDenyReason::TeredoServer),
        ),
        (
            "2001:0:808:808:0:0:f5ff:fffe",
            AddressDecision::Denied(AddressDenyReason::TeredoClient),
        ),
        (
            "64:ff9b:1::808:808",
            AddressDecision::Denied(AddressDenyReason::LocalUseNat64),
        ),
    ];

    for (raw, expected) in cases {
        assert_eq!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(raw).unwrap())),
            expected,
            "transition classification mismatch for {raw}"
        );
    }
}

#[test]
fn configured_nat64_extracts_public_and_special_ipv4_at_all_six_lengths() {
    let vectors = [
        (
            "2400:db8::",
            32,
            "2400:db8:808:808::",
            "2400:db8:c000:221::",
        ),
        (
            "2401:db8:100::",
            40,
            "2401:db8:108:808:8::",
            "2401:db8:1c0:2:21::",
        ),
        (
            "2402:db8:122::",
            48,
            "2402:db8:122:808:8:800::",
            "2402:db8:122:c000:2:2100::",
        ),
        (
            "2403:db8:122:300::",
            56,
            "2403:db8:122:308:8:808::",
            "2403:db8:122:3c0:0:221::",
        ),
        (
            "2404:db8:122:344::",
            64,
            "2404:db8:122:344:8:808:800:0",
            "2404:db8:122:344:c0:2:2100:0",
        ),
        (
            "2405:db8:122:344::",
            96,
            "2405:db8:122:344::808:808",
            "2405:db8:122:344::c000:221",
        ),
    ];

    for (prefix, length, public_candidate, special_candidate) in vectors {
        let policy = AddressPolicy::with_nat64_prefixes([ipv6_net(prefix, length)]).unwrap();
        assert_eq!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(public_candidate).unwrap())),
            AddressDecision::Allowed,
            "public RFC 6052 extraction failed for /{length}"
        );
        assert_eq!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(special_candidate).unwrap())),
            AddressDecision::Denied(AddressDenyReason::EmbeddedIpv4),
            "special RFC 6052 extraction failed for /{length}"
        );
    }
}

#[test]
fn configured_nat64_ignores_suffix_but_denies_nonzero_u_octets() {
    let policy = AddressPolicy::with_nat64_prefixes([ipv6_net("2404:db8:122:344::", 64)]).unwrap();

    for candidate in [
        "2404:db8:122:344:8:808:800:0",
        "2404:db8:122:344:8:808:8ff:ffff",
    ] {
        assert_eq!(
            policy.classify(IpAddr::V6(Ipv6Addr::from_str(candidate).unwrap())),
            AddressDecision::Allowed
        );
    }
    assert_eq!(
        policy.classify(IpAddr::V6(
            Ipv6Addr::from_str("2404:db8:122:344:108:808::").unwrap()
        )),
        AddressDecision::Denied(AddressDenyReason::Nat64UOctet)
    );
}

#[test]
fn configured_nat64_prefix_construction_is_fail_closed_and_unambiguous() {
    let cases = [
        (
            vec![ipv6_net("2400::", 24)],
            AddressPolicyError::InvalidPrefixLength,
        ),
        (
            vec![ipv6_net("2400:db8::1", 96)],
            AddressPolicyError::NonCanonical,
        ),
        (
            vec![ipv6_net("fc00::", 96)],
            AddressPolicyError::OutsideAllowedIpv6,
        ),
        (
            vec![ipv6_net("64:ff9b::", 96)],
            AddressPolicyError::SpecialRange,
        ),
        (
            vec![ipv6_net("2002::", 32)],
            AddressPolicyError::SpecialRange,
        ),
        (
            vec![ipv6_net("2400:db8:0:0:100::", 96)],
            AddressPolicyError::NonZeroUOctet,
        ),
        (
            vec![ipv6_net("2400:db8::", 32), ipv6_net("2400:db8:100::", 40)],
            AddressPolicyError::Overlap,
        ),
    ];

    for (prefixes, expected) in cases {
        assert_eq!(AddressPolicy::with_nat64_prefixes(prefixes), Err(expected));
    }

    for prefix in [
        ipv6_net("::", 96),
        ipv6_net("::ffff:0:0", 96),
        ipv6_net("64:ff9b:1::", 48),
        ipv6_net("2001::", 32),
        ipv6_net("2001:db8::", 32),
        ipv6_net("3fff::", 32),
    ] {
        assert_eq!(
            AddressPolicy::with_nat64_prefixes([prefix]),
            Err(AddressPolicyError::SpecialRange)
        );
    }
}

#[test]
fn entry_identity_matches_normative_guid_and_url_index_vectors() {
    let empty = StableEntryFields::new(None, None, None, None, None).unwrap();
    let guid = EntryIdentity::from_parts(
        Some("tag:example.com,2026:42"),
        Some("https://ignored.example/post"),
        empty.clone(),
    )
    .unwrap();
    assert_eq!(guid.kind(), IdentityKind::Guid);
    assert_eq!(guid.kind().as_database_str(), "GUID");
    assert_eq!(guid.identity(), "tag:example.com,2026:42");
    assert_eq!(
        guid.index_bytes_v1(),
        bytes_from_hex("52444958000101000000177461673a6578616d706c652e636f6d2c323032363a3432")
    );
    assert_eq!(
        guid.index_hash(),
        "e697b4d9b1ce018d8e0ed595b79680c41fdde20685bac778a96724b6380cc13f"
    );

    let url = EntryIdentity::from_parts(
        Some("HTTPS://EXAMPLE.com:443/post?a=1&a=2#fragment"),
        None,
        empty,
    )
    .unwrap();
    assert_eq!(url.kind(), IdentityKind::Url);
    assert_eq!(url.kind().as_database_str(), "URL");
    assert_eq!(url.identity(), "https://example.com/post?a=1&a=2");
    assert_eq!(
        url.index_bytes_v1(),
        bytes_from_hex(
            "524449580001020000002068747470733a2f2f6578616d706c652e636f6d2f706f73743f613d3126613d32"
        )
    );
    assert_eq!(
        url.index_hash(),
        "2d2ef85d39b2644f36462c9e4dd119525683d248e66273bcaea04000f8ad9857"
    );
}

#[test]
fn stable_entry_fields_match_normative_ordinary_fingerprint_vector() {
    let fields = StableEntryFields::new(
        Some("Hello world"),
        Some("Alice"),
        Some(1_700_000_000_123_456),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        fields.encode_v1(),
        bytes_from_hex(
            "52444650000101010000000b48656c6c6f20776f726c64020100000005416c69636503010000000800060a2418202240040000000000050000000000"
        )
    );

    let identity = EntryIdentity::from_parts(None, None, fields).unwrap();
    assert_eq!(identity.kind(), IdentityKind::Fingerprint);
    assert_eq!(identity.kind().as_database_str(), "FINGERPRINT");
    assert_eq!(
        identity.identity(),
        "a0b3e922878ae50dae0e706b4a43aea8438d740ff478a047eb05e869296160ed"
    );
    assert_eq!(
        identity.index_hash(),
        "2ecb60b16bd41841bd5403adeb3a146ce9304a1fb904a26ac4549849b762788e"
    );
}

#[test]
fn stable_entry_fields_match_normative_content_only_fingerprint_vector() {
    let fields = StableEntryFields::new(None, None, None, None, Some([0x11; 32])).unwrap();
    assert_eq!(
        fields.encode_v1(),
        bytes_from_hex(
            "5244465000010100000000000200000000000300000000000400000000000501000000201111111111111111111111111111111111111111111111111111111111111111"
        )
    );

    let identity = EntryIdentity::from_parts(None, None, fields).unwrap();
    assert_eq!(
        identity.identity(),
        "a483e2c16d043f36aa56e3a6c203a76e5e340a4f0713a2632d8cb9f7b1cfc0e3"
    );
    assert_eq!(
        identity.index_hash(),
        "393eaf5d121df51cc2f4817011fd3e0d593da32a6ac542663c354ce08985d65d"
    );
}

#[test]
fn entry_identity_uses_guid_then_canonical_url_then_fingerprint() {
    let fields = StableEntryFields::new(Some("title"), None, None, None, None).unwrap();
    let guid = EntryIdentity::from_parts(
        Some("  opaque GUID  "),
        Some("https://example.com/canonical"),
        fields.clone(),
    )
    .unwrap();
    assert_eq!(guid.kind(), IdentityKind::Guid);
    assert_eq!(guid.identity(), "opaque GUID");

    let url = EntryIdentity::from_parts(
        Some("\u{2003}\t"),
        Some("HTTP://EXAMPLE.com:80/post?x=1&x=2#ignored"),
        fields.clone(),
    )
    .unwrap();
    assert_eq!(url.kind(), IdentityKind::Url);
    assert_eq!(url.identity(), "http://example.com/post?x=1&x=2");

    let fingerprint = EntryIdentity::from_parts(None, None, fields).unwrap();
    assert_eq!(fingerprint.kind(), IdentityKind::Fingerprint);
}

#[test]
fn entry_identity_rejects_credential_like_url_guids_without_leaking_them() {
    for raw in [
        "https://publisher:top-secret@example.com:bad/post?token=also-secret",
        "https:/publisher:top-secret@example.com/feed",
        r"HtTp:\publisher:top-secret@example.com/feed",
        "https:///publisher:top-secret@example.com/feed",
        "https:////publisher:top-secret@example.com/feed",
        r"https://\publisher:top-secret@example.com/feed",
        "https:///@example.com/feed",
        "https:////@example.com/feed",
    ] {
        let error = EntryIdentity::from_parts(
            Some(raw),
            None,
            StableEntryFields::new(None, None, None, None, None).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error, IdentityError::CredentialsForbidden);
        let rendered = format!("{error:?}: {error}");
        for secret in [raw, "publisher", "top-secret", "also-secret"] {
            assert!(!rendered.contains(secret));
        }
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn invalid_noncredential_url_like_guid_remains_opaque() {
    let raw = "https://example.com:bad/post";
    let identity = EntryIdentity::from_parts(
        Some(raw),
        None,
        StableEntryFields::new(None, None, None, None, None).unwrap(),
    )
    .unwrap();
    assert_eq!(identity.kind(), IdentityKind::Guid);
    assert_eq!(identity.identity(), raw);
}

#[test]
fn stable_text_normalization_is_unicode_aware_but_not_case_folding() {
    let normalized = StableEntryFields::new(
        Some(" \tHello\u{2003}\nworld  "),
        Some(" ALICE\u{a0}\u{a0}Smith "),
        None,
        None,
        Some([0x22; 32]),
    )
    .unwrap();
    let expected =
        StableEntryFields::new(Some("Hello world"), Some("ALICE Smith"), None, None, None).unwrap();
    assert_eq!(normalized, expected);

    let empty = StableEntryFields::new(Some("\u{2003}\t"), None, None, None, None).unwrap();
    let absent = StableEntryFields::new(None, None, None, None, None).unwrap();
    assert_eq!(empty, absent);

    let debug = format!(
        "{:?}",
        StableEntryFields::new(
            Some("private title"),
            Some("secret author"),
            None,
            Some("https://example.com/audio?token=top-secret"),
            None,
        )
        .unwrap()
    );
    for secret in ["private title", "secret author", "token", "top-secret"] {
        assert!(!debug.contains(secret));
    }
}

#[test]
fn stable_entry_field_frames_prevent_concatenation_ambiguity() {
    let first = StableEntryFields::new(Some("ab"), Some("c"), None, None, None).unwrap();
    let second = StableEntryFields::new(Some("a"), Some("bc"), None, None, None).unwrap();
    assert_ne!(first.encode_v1(), second.encode_v1());
    assert_ne!(
        EntryIdentity::from_parts(None, None, first).unwrap(),
        EntryIdentity::from_parts(None, None, second).unwrap()
    );
}

#[test]
fn stable_entry_inputs_enforce_boundaries_and_normalize_enclosure_urls() {
    assert!(StableEntryFields::new(Some(&"a".repeat(65_536)), None, None, None, None).is_ok());
    assert_eq!(
        StableEntryFields::new(Some(&"a".repeat(65_537)), None, None, None, None),
        Err(IdentityError::TooLong)
    );
    assert_eq!(
        EntryIdentity::from_parts(
            Some(&"g".repeat(65_537)),
            None,
            StableEntryFields::new(None, None, None, None, None).unwrap(),
        ),
        Err(IdentityError::TooLong)
    );

    let first = StableEntryFields::new(
        None,
        None,
        None,
        Some("HTTPS://EXAMPLE.com:443/audio.mp3?x=1&x=2#fragment"),
        None,
    )
    .unwrap();
    let equivalent = StableEntryFields::new(
        None,
        None,
        None,
        Some("https://example.com/audio.mp3?x=1&x=2"),
        None,
    )
    .unwrap();
    assert_eq!(first, equivalent);
}

#[test]
fn fallback_identity_changes_when_any_best_effort_input_changes() {
    let baseline = EntryIdentity::from_parts(
        None,
        None,
        StableEntryFields::new(
            Some("title"),
            Some("author"),
            Some(42),
            Some("https://example.com/enclosure"),
            None,
        )
        .unwrap(),
    )
    .unwrap();
    let changed = [
        StableEntryFields::new(
            Some("title corrected"),
            Some("author"),
            Some(42),
            Some("https://example.com/enclosure"),
            None,
        )
        .unwrap(),
        StableEntryFields::new(
            Some("title"),
            Some("another author"),
            Some(42),
            Some("https://example.com/enclosure"),
            None,
        )
        .unwrap(),
        StableEntryFields::new(
            Some("title"),
            Some("author"),
            Some(43),
            Some("https://example.com/enclosure"),
            None,
        )
        .unwrap(),
        StableEntryFields::new(
            Some("title"),
            Some("author"),
            Some(42),
            Some("https://example.com/other"),
            None,
        )
        .unwrap(),
    ];
    for fields in changed {
        assert_ne!(
            baseline,
            EntryIdentity::from_parts(None, None, fields).unwrap()
        );
    }

    let content_a = EntryIdentity::from_parts(
        None,
        None,
        StableEntryFields::new(None, None, None, None, Some([1; 32])).unwrap(),
    )
    .unwrap();
    let content_b = EntryIdentity::from_parts(
        None,
        None,
        StableEntryFields::new(None, None, None, None, Some([2; 32])).unwrap(),
    )
    .unwrap();
    assert_ne!(content_a, content_b);
}

#[test]
fn entry_identity_debug_redacts_url_guid_and_identity_text() {
    let identity = EntryIdentity::from_parts(
        Some("https://example.com/private?token=top-secret"),
        None,
        StableEntryFields::new(None, None, None, None, None).unwrap(),
    )
    .unwrap();
    let debug = format!("{identity:?}");
    for secret in ["private", "token", "top-secret", identity.identity()] {
        assert!(!debug.contains(secret));
    }
}

#[test]
fn opaque_validator_round_trips_non_utf8_bytes_with_canonical_storage() {
    let validator =
        OpaqueValidator::from_header(HeaderValue::from_bytes(&[0xff]).unwrap()).unwrap();
    assert_eq!(validator.storage_value(), "v1:_w");
    assert_eq!(validator.header_value().as_bytes(), &[0xff]);
    assert!(validator.header_value().is_sensitive());

    let reconstructed = OpaqueValidator::from_storage("v1:_w").unwrap();
    assert_eq!(reconstructed.header_value().as_bytes(), &[0xff]);
    assert!(reconstructed.header_value().is_sensitive());
    assert!(reconstructed.clone().header_value().is_sensitive());
    assert_eq!(reconstructed.storage_value(), "v1:_w");
}

#[test]
fn opaque_validator_enforces_header_byte_boundaries() {
    let one = OpaqueValidator::from_header(HeaderValue::from_bytes(b"x").unwrap()).unwrap();
    assert_eq!(one.header_value().as_bytes(), b"x");

    let maximum_bytes = vec![b'x'; 8_192];
    let maximum =
        OpaqueValidator::from_header(HeaderValue::from_bytes(&maximum_bytes).unwrap()).unwrap();
    assert_eq!(maximum.header_value().as_bytes().len(), 8_192);

    assert_eq!(
        OpaqueValidator::from_header(HeaderValue::from_static("")),
        Err(ValidatorError::Empty)
    );
    let oversized = vec![b'x'; 8_193];
    assert_eq!(
        OpaqueValidator::from_header(HeaderValue::from_bytes(&oversized).unwrap()),
        Err(ValidatorError::TooLong)
    );

    let oversized_storage = format!(
        "v1:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(oversized)
    );
    assert_eq!(
        OpaqueValidator::from_storage(&oversized_storage),
        Err(ValidatorError::TooLong)
    );
}

#[test]
fn opaque_validator_rejects_unknown_corrupt_and_noncanonical_storage() {
    let cases = [
        ("v2:_w", ValidatorError::UnsupportedVersion),
        ("_w", ValidatorError::UnsupportedVersion),
        ("v1:", ValidatorError::Empty),
        ("v1:_w==", ValidatorError::InvalidEncoding),
        ("v1: _w", ValidatorError::InvalidEncoding),
        ("v1:/w", ValidatorError::InvalidEncoding),
        ("v1:_x", ValidatorError::InvalidEncoding),
        ("v1:Cg", ValidatorError::InvalidHeaderValue),
    ];

    for (storage, expected) in cases {
        let error = OpaqueValidator::from_storage(storage).unwrap_err();
        assert_eq!(error, expected, "unexpected error for storage category");
        assert!(!format!("{error:?}: {error}").contains(storage));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn opaque_validator_rejects_oversized_encoded_storage_before_decoding() {
    let storage = format!("v1:{}", "A".repeat(10_925));
    assert_eq!(
        OpaqueValidator::from_storage(&storage),
        Err(ValidatorError::TooLong)
    );
}

#[test]
fn validator_set_is_reusable_only_for_the_exact_complete_normalized_url() {
    let policy = FeedUrlPolicy::new(false);
    let original = policy
        .normalize("https://example.com/feed?token=secret&x=1")
        .unwrap();
    let exact = policy
        .normalize("HTTPS://EXAMPLE.com:443/feed?token=secret&x=1#ignored")
        .unwrap();
    let changed_query = policy
        .normalize("https://example.com/feed?token=other&x=1")
        .unwrap();
    let changed_final_url = policy
        .normalize("https://example.com/redirected?token=secret&x=1")
        .unwrap();
    let changed_origin = policy
        .normalize("https://other.example/feed?token=secret&x=1")
        .unwrap();

    let validators = ValidatorSet::new(
        &original,
        Some(OpaqueValidator::from_header(HeaderValue::from_static("opaque-etag")).unwrap()),
        Some(
            OpaqueValidator::from_header(HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"))
                .unwrap(),
        ),
    );

    let reusable = validators.for_request(&exact).unwrap();
    assert_eq!(reusable.etag().unwrap().as_bytes(), b"opaque-etag");
    assert!(reusable.etag().unwrap().is_sensitive());
    assert!(reusable.last_modified().unwrap().is_sensitive());
    for changed in [&changed_query, &changed_final_url, &changed_origin] {
        assert!(validators.for_request(changed).is_none());
    }
}

#[test]
fn validator_debug_formatting_redacts_url_query_and_header_bytes() {
    let validator =
        OpaqueValidator::from_header(HeaderValue::from_static("top-secret-etag")).unwrap();
    let url = FeedUrlPolicy::new(false)
        .normalize("https://example.com/private?token=top-secret-query")
        .unwrap();
    let set = ValidatorSet::new(&url, Some(validator.clone()), None);

    for debug in [format!("{validator:?}"), format!("{set:?}")] {
        for secret in ["top-secret-etag", "private", "token", "top-secret-query"] {
            assert!(!debug.contains(secret));
        }
    }
}

#[test]
fn refresh_schedule_success_and_not_modified_reset_exactly_to_five_minutes() {
    let now = utc(2026, Month::July, 17, 12, 0, 0);
    for result in [RefreshResult::Success, RefreshResult::NotModified] {
        let mut schedule = RefreshSchedule::new(FixedJitter::new([]));
        let outcome = schedule.after_result(now, i64::MAX, result).unwrap();
        assert_eq!(outcome.next_at(), now + Duration::minutes(5));
        assert_eq!(outcome.consecutive_failures(), 0);
        assert_eq!(outcome.retry_after_at(), None);
    }
}

#[test]
fn refresh_schedule_validates_negative_persisted_counts_for_every_result() {
    let now = utc(2026, Month::July, 17, 12, 0, 0);
    let results = [
        RefreshResult::Success,
        RefreshResult::NotModified,
        RefreshResult::TransientFailure { retry_after: None },
    ];
    for result in results {
        let mut schedule = RefreshSchedule::new(FixedJitter::new([]));
        assert_eq!(
            schedule.after_result(now, -1, result),
            Err(ScheduleError::NegativeFailureCount)
        );
    }
}

#[test]
fn refresh_schedule_owns_increment_saturation_and_all_backoff_bounds() {
    let now = utc(2026, Month::July, 17, 12, 0, 0);
    let cases = [
        (0, 300_000_000_u64, 1_i64),
        (5, 9_600_000_000, 6),
        (6, 14_400_000_000, 7),
        (i64::MAX, 14_400_000_000, i64::MAX),
    ];
    for (previous, upper_bound_us, expected_count) in cases {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut schedule = RefreshSchedule::new(RecordingJitter {
            observed: Rc::clone(&observed),
            return_upper_bound: true,
        });
        let outcome = schedule
            .after_result(
                now,
                previous,
                RefreshResult::TransientFailure { retry_after: None },
            )
            .unwrap();
        assert_eq!(&*observed.borrow(), &[upper_bound_us]);
        assert_eq!(
            outcome.next_at(),
            now + Duration::microseconds(upper_bound_us as i64)
        );
        assert_eq!(outcome.consecutive_failures(), expected_count);
    }
}

#[test]
fn refresh_schedule_full_jitter_is_inclusive_and_rejects_out_of_range_sources() {
    let now = utc(2026, Month::July, 17, 12, 0, 0);
    for (sample, expected) in [(0, now), (300_000_000, now + Duration::minutes(5))] {
        let mut schedule = RefreshSchedule::new(FixedJitter::new([sample]));
        let outcome = schedule
            .after_result(
                now,
                0,
                RefreshResult::TransientFailure { retry_after: None },
            )
            .unwrap();
        assert_eq!(outcome.next_at(), expected);
    }

    let mut invalid = RefreshSchedule::new(FixedJitter::new([300_000_001]));
    assert_eq!(
        invalid.after_result(
            now,
            0,
            RefreshResult::TransientFailure { retry_after: None }
        ),
        Err(ScheduleError::InvalidJitter)
    );
}

#[test]
fn retry_after_parses_delta_seconds_and_all_compatible_http_dates() {
    let received_at = utc(1994, Month::November, 6, 8, 47, 37);
    let delta =
        RetryAfter::parse(&HeaderValue::from_bytes(b" \t120\t ").unwrap(), received_at).unwrap();
    assert_eq!(delta.at(), utc(1994, Month::November, 6, 8, 49, 37));

    for raw in [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
    ] {
        let parsed = RetryAfter::parse(&HeaderValue::from_str(raw).unwrap(), received_at).unwrap();
        assert_eq!(parsed.at(), utc(1994, Month::November, 6, 8, 49, 37));
    }
}

#[test]
fn retry_after_rejects_non_ascii_invalid_and_overflowing_syntax() {
    let received_at = utc(2026, Month::July, 17, 12, 0, 0);
    let cases = [
        (HeaderValue::from_static(""), RetryAfterError::Empty),
        (
            HeaderValue::from_static("12seconds"),
            RetryAfterError::Invalid,
        ),
        (
            HeaderValue::from_bytes(&[0xff]).unwrap(),
            RetryAfterError::Invalid,
        ),
        (
            HeaderValue::from_static("18446744073709551616"),
            RetryAfterError::DeltaOverflow,
        ),
    ];
    for (raw, expected) in cases {
        let error = RetryAfter::parse(&raw, received_at).unwrap_err();
        assert_eq!(error, expected);
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn retry_after_delta_is_receipt_anchored_and_saturates_valid_add_overflow() {
    let received_at = utc(2026, Month::July, 17, 12, 0, 0);
    let retry = RetryAfter::parse(&HeaderValue::from_static("120"), received_at).unwrap();
    let now = received_at + Duration::seconds(30);
    let mut schedule = RefreshSchedule::new(FixedJitter::new([0]));
    let outcome = schedule
        .after_result(
            now,
            0,
            RefreshResult::TransientFailure {
                retry_after: Some(retry),
            },
        )
        .unwrap();
    assert_eq!(outcome.next_at(), now + Duration::seconds(90));
    assert_eq!(
        outcome.retry_after_at(),
        Some(received_at + Duration::seconds(120))
    );

    let maximum = PrimitiveDateTime::MAX.assume_utc();
    let saturated = RetryAfter::parse(
        &HeaderValue::from_static("2"),
        maximum - Duration::seconds(1),
    )
    .unwrap();
    assert_eq!(saturated.at(), maximum);
    let huge = RetryAfter::parse(
        &HeaderValue::from_static("9223372036854775808"),
        received_at,
    )
    .unwrap();
    assert_eq!(huge.at(), maximum);
}

#[test]
fn refresh_schedule_uses_retry_after_floor_past_zero_and_four_hour_cap() {
    let now = utc(2026, Month::July, 17, 12, 0, 0);
    let past = RetryAfter::parse(
        &HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        now,
    )
    .unwrap();
    let mut with_past = RefreshSchedule::new(FixedJitter::new([10_000_000]));
    let past_outcome = with_past
        .after_result(
            now,
            0,
            RefreshResult::TransientFailure {
                retry_after: Some(past),
            },
        )
        .unwrap();
    assert_eq!(past_outcome.next_at(), now + Duration::seconds(10));

    let future = RetryAfter::parse(&HeaderValue::from_static("36000"), now).unwrap();
    let mut capped = RefreshSchedule::new(FixedJitter::new([0]));
    let capped_outcome = capped
        .after_result(
            now,
            0,
            RefreshResult::TransientFailure {
                retry_after: Some(future),
            },
        )
        .unwrap();
    assert_eq!(capped_outcome.next_at(), now + Duration::hours(4));
    assert_eq!(
        capped_outcome.retry_after_at(),
        Some(now + Duration::hours(10))
    );
}

#[test]
fn refresh_schedule_reports_checked_next_time_overflow() {
    let maximum = PrimitiveDateTime::MAX.assume_utc();
    let mut success = RefreshSchedule::new(FixedJitter::new([]));
    assert_eq!(
        success.after_result(maximum - Duration::minutes(4), 0, RefreshResult::Success),
        Err(ScheduleError::TimeOverflow)
    );
}

#[derive(Debug)]
struct FixedJitter {
    samples: VecDeque<u64>,
}

impl FixedJitter {
    fn new(samples: impl IntoIterator<Item = u64>) -> Self {
        Self {
            samples: samples.into_iter().collect(),
        }
    }
}

impl JitterSource for FixedJitter {
    fn sample_inclusive_us(&mut self, _upper_bound_us: u64) -> u64 {
        self.samples.pop_front().expect("unexpected jitter sample")
    }
}

struct RecordingJitter {
    observed: Rc<RefCell<Vec<u64>>>,
    return_upper_bound: bool,
}

impl JitterSource for RecordingJitter {
    fn sample_inclusive_us(&mut self, upper_bound_us: u64) -> u64 {
        self.observed.borrow_mut().push(upper_bound_us);
        if self.return_upper_bound {
            upper_bound_us
        } else {
            0
        }
    }
}

fn utc(year: i32, month: Month, day: u8, hour: u8, minute: u8, second: u8) -> OffsetDateTime {
    PrimitiveDateTime::new(
        Date::from_calendar_date(year, month, day).unwrap(),
        Time::from_hms(hour, minute, second).unwrap(),
    )
    .assume_utc()
}

fn ipv6_net(address: &str, prefix_len: u8) -> Ipv6Net {
    Ipv6Net::new(Ipv6Addr::from_str(address).unwrap(), prefix_len).unwrap()
}

fn bytes_from_hex(raw: &str) -> Vec<u8> {
    raw.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap();
            let low = (pair[1] as char).to_digit(16).unwrap();
            ((high << 4) | low) as u8
        })
        .collect()
}
