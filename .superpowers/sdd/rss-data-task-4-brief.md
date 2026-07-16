### Task 4: Pinned, bounded HTTP fetch transport

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/feeds/resolver.rs`
- Create: `src/feeds/fetch.rs`
- Create: `src/feeds/decode.rs`
- Create: `src/feeds/test_support.rs`
- Modify: `src/feeds/mod.rs`

**Interfaces:**

- Add `reqwest = { version = "0.13.4", default-features = false, features = ["rustls-no-provider", "stream"] }` and `rustls = { version = "0.23.42", default-features = false, features = ["ring"] }`. The application installs the ring `CryptoProvider` immediately after the `--version` fast path and before tracing/config/database work; installation is idempotent for the exact ring provider and a conflicting process provider fails hard. The production executor repeats the idempotent check before every reqwest client build. AWS-LC and `aws-lc-sys` must not be reachable in the locked graph.
- Add `async-trait = "0.1.89"`, `async-compression = { version = "0.4.42", default-features = false, features = ["tokio", "gzip", "zlib", "brotli"] }`, `futures-util = "0.3.32"`, and `hickory-resolver = { version = "=0.26.1", default-features = false, features = ["system-config", "tokio"] }`. Hickory 0.26.1 has MSRV 1.88 and supplies explicit record-type lookup, DNS message/rcode inspection, and `Lookup::valid_until`; no DNS-over-TLS/HTTPS/QUIC/DNSSEC feature is enabled. Add Tokio production features `sync` and `time`; add Tokio `test-util` for unit tests without changing production behavior.
- `#[async_trait::async_trait] trait DnsResolver: Send + Sync` returns all explicit A/AAAA `IpAddr` values for attacker-controlled hosts under a 3-second deadline. Production uses the system-configured Hickory resolver; tests inject a fake resolver. An absent family is accepted only for `NoRecordsFound` with `response_code == NoError` (NODATA); NXDOMAIN and every other rcode fail closed.
- A separate `#[async_trait::async_trait] trait Nat64PrefixDiscovery: Send + Sync` returns typed `Present`, `NotPresent`, or error plus a monotonic validity deadline from explicit `ipv4only.arpa.` A/AAAA queries. Its dedicated system-configured Hickory resolver uses `ResolveHosts::Never`, `cache_size=0`, `attempts=1`, leaves all positive/negative TTL min/max options unset, and never shares the user-host resolver cache. A and AAAA execute concurrently inside one three-second timeout and any fetch-triggered refresh is also bounded by the remaining 30-second total deadline. Both outcomes require an A `NOERROR` answer containing both WKAs. `Present` uses the minimum A/AAAA `Lookup::valid_until()`; `NotPresent` requires `NetError::Dns(DnsError::NoRecordsFound(no_records))` with `response_code == NoError` and `negative_ttl == Some(nonzero)`, and uses the minimum A deadline and checked `now + negative_ttl`. NXDOMAIN, missing/zero TTL, overflow, malformed answers, and every other response are errors. Automatic, static-prefix, and explicit-disabled modes follow the Task 3 state machine. The production transport owns one atomic `{ generation, valid_until, address_policy }` snapshot; `now >= valid_until` is expired, and the first fetch at/after expiry refreshes single-flight before any user-controlled DNS. A failed expired refresh blocks the fetch. Same-policy TTL renewal may preserve generation while publishing a new deadline; policy changes increment generation.
- Public `#[async_trait::async_trait] trait FeedTransport: Send + Sync { async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError>; }` is the injection seam used by later domain/integration tests. Production `HttpFeedTransport` owns private async `DnsResolver` and per-hop async `HttpExecutor` seams; unit tests inject fakes without exposing a runtime loopback bypass.
- `FetchOutcome::Document` owns the final normalized response URL, bounded decoded body bytes, an optional single validated UTF-8 `Content-Type` string for Task 5 MIME/charset preflight, and optional response `OpaqueValidator` ETag/Last-Modified values. `FetchOutcome::NotModified` owns the final URL and optional response validators so a 304 can update metadata without polling a body. Multiple `Content-Type`, ETag, or Last-Modified fields and a non-UTF-8 Content-Type are typed response errors. Public accessors expose owned typed metadata; Debug prints only the redacted URL view, byte count, and field presence.
- Each hop builds a short-lived reqwest client with `redirect(Policy::none())`, `no_proxy()`, `no_gzip()`, `no_brotli()`, `no_deflate()`, `no_zstd()`, 5-second connect timeout, 10-second read-idle timeout, remaining hop/total deadlines, and `resolve_to_addrs(host, approved_socket_addrs)`.
- At most 16 DNS results and five redirects. Any denied address rejects the whole set. Every redirect repeats URL/DNS/address validation and never forwards validators across a changed validator URL.
- The 16-address limit applies to the raw resolver result before deduplication: zero or more than 16 rejects, otherwise exact duplicate `IpAddr`s are removed before socket construction. Five redirect responses are allowed after the initial request; a sixth redirect response rejects. A successful compressed ratio is inclusive at `decoded_len <= compressed_len * 100`; the next decoded byte rejects.
- A successful response must have `remote_addr()` in the approved address set. Missing/mismatched peer information rejects before body processing.
- Absolute deadlines cover DNS 3 s, connect 5 s, first byte 10 s, each body-idle interval 10 s, one HTTP request/redirect hop 20 s, decoding, and the whole refresh 30 s. Every deadline gate rejects `now >= deadline` both before awaiting and after a future reports ready, so completion exactly at a deadline fails; ties prefer Total, then Hop, then the operation stage. A new redirect target starts a new 20-second hop deadline, while DNS replays within that hop do not; no redirect or replay resets the 30-second total deadline. Bounded in-memory decoding cooperatively yields between output chunks so its hop/total timeout can preempt ready async-compression loops. Reqwest body timeout errors preserve `is_timeout()` and are classified from the current Total/Hop/BodyIdle deadlines instead of becoming generic network failures.
- Automatic decompression is disabled. Compressed bytes are capped at 2 MiB; decoded bytes at 10 MiB; ratio at 100:1. `Content-Encoding` accepts only: no field (identity), or exactly one field whose OWS-trimmed bytes case-insensitively equal `identity`, `gzip`, `br`, or `deflate`. Empty values, multiple header fields, comma lists/layering, parameters, non-ASCII bytes, and every other token reject with a typed encoding error. HTTP `deflate` means zlib-wrapped DEFLATE in v1; raw DEFLATE returns a typed decode error. Gzip processes all members under the same decoded/ratio budgets.
- Status handling is exact: `200` yields the bounded decoded document; `304` yields `NotModified` without polling a body; only `301/302/303/307/308` redirect and require exactly one valid `Location`; every other status returns a typed status error without reading the body. If a single valid `Retry-After` is present on that status, the transport parses and preserves it relative to the response receipt time; multiple or invalid values are typed response errors.
- `FeedFetchError` strips reqwest URLs with `without_url()` before storing a source. Custom `Debug` output contains only error class, host, and counts.

- [ ] **Step 1: Write failing transport abuse tests**

Place abuse tests inside `resolver.rs`, `fetch.rs`, and `decode.rs` unit-test modules. `test_support.rs` is compiled only under `#[cfg(test)]` and contains fake resolver/executor/scripted server utilities. No integration test or production API receives a loopback-allowing policy.

Required test names:

```rust
mixed_public_and_private_dns_answers_fail_before_connect()
resolver_is_called_once_per_hop_and_pinned_addresses_are_used()
redirects_revalidate_private_targets_and_stop_after_five_hops()
https_redirect_cannot_downgrade_to_http()
validators_are_sent_only_to_the_exact_validator_url()
not_modified_does_not_read_or_parse_a_body()
nat64_absence_requires_verified_a_and_nodata_aaaa()
nat64_discovery_covers_all_six_prefix_lengths_and_multiple_prefixes()
nat64_discovery_rejects_ambiguous_wka_mappings_and_negative_responses()
nat64_snapshot_refreshes_by_ttl_and_blocks_fetch_when_expired()
nat64_discovery_completes_before_user_controlled_dns()
nat64_snapshot_change_during_user_dns_discards_and_reresolves()
nat64_snapshot_change_before_executor_discards_and_reresolves()
nat64_snapshot_replay_exhaustion_is_typed_and_bounded()
nat64_discovery_bypasses_hosts_and_response_cache()
nat64_a_and_aaaa_share_one_three_second_deadline()
compressed_and_decoded_limits_stop_streaming()
brotli_gzip_and_deflate_decode_within_budget()
timeouts_share_one_total_refresh_deadline()
first_byte_timeout_is_distinct_from_body_idle_timeout()
body_idle_timeout_resets_only_after_a_chunk()
redirects_reset_hop_but_not_total_deadline()
connected_peer_must_match_the_approved_set()
reqwest_errors_do_not_expose_url_queries_in_debug_or_display()
```

- [ ] **Step 2: Add the smallest compiling transport/error scaffold**

Define `FeedTransport`, `FetchRequest`, `FetchOutcome`, typed errors, private resolver/executor traits, and an `HttpFeedTransport` constructor. The first behavior test must compile and fail on an assertion, not on a missing module or symbol.

- [ ] **Step 3: Implement one RED/GREEN behavior slice at a time**

Implement in this order, running the named unit test after each slice: dedicated no-hosts/no-cache Hickory discovery resolver and concurrent explicit A/AAAA seam; Nat64 automatic/static/disabled modes; verified A plus checked negative-TTL AAAA absence; all six RFC 7050 prefix derivations and multiple prefixes; malformed/negative/ambiguous discovery; atomic generation/deadline snapshot publication; TTL single-flight refresh and exact expiry gate; discovery-before-user-DNS ordering; generation/deadline recheck after user DNS and before executor with one shared two-replay counter plus typed exhaustion; raw-result-count then deduplicated user-host DNS classify/pin; one-hop 200/304/status handling; peer verification; manual 301/302/303/307/308; validator scoping; first-byte/body-idle/per-hop/total deadlines; exact Content-Encoding grammar; raw byte cap; multi-member gzip; Brotli; zlib-wrapped deflate; ratio/decoded cap; error redaction. Do not enable reqwest `default`, `system-proxy`, `cookies`, compression, native TLS, HTTP/2, or HTTP/3 features. `Location` may be relative and is resolved against the current URL before full revalidation.

Decoded bodies remain bytes; do not create an unbounded `String`. Invalid header values are not reflected in errors. Diagnostics contain feed ID/host/status/counts only, never full URL query, validator, or body.

- [ ] **Step 4: Audit dependency features and run transport tests**

```bash
cargo tree --locked -e features -i reqwest
cargo tree --locked -e features -i rustls
cargo tree --locked -e features -i hickory-resolver
! cargo tree --locked -i aws-lc-rs
! cargo tree --locked -i aws-lc-sys
! cargo tree --locked -e features | rg 'system-proxy|cookies|native-tls|http2|http3'
! cargo tree --locked -e features | rg 'hickory-resolver feature "(dnssec|h3|https|quic|tls)[^"]*"'
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked feeds:: -- --nocapture
```

Expected: PASS; tree shows reqwest `rustls-no-provider`/stream, one rustls ring provider, and only Brotli/gzip/zlib codecs, without AWS-LC, system proxy, cookie jar, native-tls, or HTTP/3.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/feeds
git commit -m "feat: fetch feeds through pinned transport"
```

---
