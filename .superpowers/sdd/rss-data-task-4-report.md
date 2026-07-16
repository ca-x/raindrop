# RSS data Task 4 implementation report

## Status

Implemented the pinned, bounded feed HTTP transport from the regenerated Task 4
brief and preflight, using `672f03d` as the implementation/review base. The
initial change was limited to the authorized dependency files, `src/feeds`
transport files, and this report. The explicitly authorized post-review wave
also updates the binary's early TLS initialization and the Task 4 canonical and
generated contract documents. The untracked root `node_modules/` directory was
not touched.

## TDD evidence

The first resolver slice was driven through the named
`nat64_discovery_bypasses_hosts_and_response_cache` test.

```text
RED: the default Hickory builder reported ResolveHosts::Auto
GREEN: the dedicated discovery builder reported ResolveHosts::Never,
       cache_size=0, attempts=1, and unset positive/negative TTL clamps
```

The implementation then proceeded through the required resolver, snapshot,
pinning, response, redirect, deadline, streaming, codec, and redaction seams.
The final feed suite has 32 tests, including every required test name plus
additional coverage for DNS raw cardinality, IP literals, concurrent NAT64
single-flight, shared replay DNS deadlines, connect-timeout classification,
ambiguous response metadata, Retry-After, and exact Location cardinality.

## Implementation

- Added the exact no-default-feature reqwest, Hickory, async-compression,
  async-trait, and futures-util dependencies and the required Tokio features.
- Added separate system-configured Hickory resolvers for user hostnames and RFC
  7050 discovery. Discovery bypasses hosts/cache, queries explicit absolute A
  and AAAA records concurrently, verifies both WKAs, validates positive and
  negative TTLs, and derives all six RFC 6052 prefix lengths.
- Added automatic, static-prefix, and explicitly disabled NAT64 snapshot modes.
  Automatic refresh is on-demand and single-flight; policy/deadline publication
  is atomic through an immutable `Arc` snapshot behind a lock. Exact expiry,
  same-policy TTL renewal, generation changes, failed refresh blocking, and both
  stale snapshot gates are covered.
- Added the public `FeedTransport` seam, owned `FetchRequest` and
  `FetchOutcome`, production `HttpFeedTransport`, typed redacted errors, and
  private resolver/executor/body test seams. Outcomes preserve the final
  normalized URL, bounded bytes, validated UTF-8 Content-Type, and optional
  response validators for both 200 and 304.
- Added raw DNS count validation before exact deduplication, all-address policy
  classification, IP-literal classification without DNS, pinned socket
  construction, root-dot-free reqwest override keys, and full peer socket
  verification before response processing.
- Added short-lived no-proxy/no-redirect/no-auto-codec reqwest clients, exact
  validator scoping, five-response redirect handling, relative Location
  resolution, HTTPS downgrade rejection, and exact status/Retry-After grammar.
- Added one total deadline, per-hop deadlines, one shared user-DNS deadline
  across snapshot replays, distinct connect/first-byte/body-idle stages, and an
  external body idle gate that resets only after non-empty data.
- Added bounded raw streaming, exact Content-Encoding parsing, multi-member
  gzip, Brotli, zlib-wrapped HTTP deflate, raw-DEFLATE rejection, a 2 MiB
  compressed cap, 10 MiB decoded cap, and inclusive 100:1 expansion limit. The
  decoder reads at most the first byte beyond an applicable decoded limit.
- Reqwest errors are stripped with `without_url()` at every storage boundary.
  Transport error Debug contains only class, canonical host, and count; outcome
  Debug contains only the redacted URL view, byte count, and metadata presence.

## Initial `4bd66c9` verification

All commands were run from
`/home/czyt/code/rust/raindrop/.worktrees/foundation-bootstrap`.

```text
cargo tree --locked -e features -i reqwest
PASS at `4bd66c9`: reqwest rustls/stream before the provider hardening below

cargo tree --locked -e features -i hickory-resolver
PASS: only system-config/tokio

forbidden reqwest/global feature grep
PASS: no system-proxy, cookies, native-tls, http2, or http3

forbidden Hickory feature grep
PASS: no dnssec, h3, https, quic, or tls feature

cargo fmt --check
PASS

cargo clippy --locked --all-targets --all-features -- -D warnings
PASS

cargo test --locked feeds:: -- --nocapture
PASS: 32 passed, 0 failed

cargo test --locked --all-features
PASS: 118 passed, 0 failed across unit/integration/doc targets

cargo +1.94.0 test --locked feeds:: -- --nocapture
PASS: 32 passed, 0 failed
```

Cargo emitted the repository's existing future-incompatibility notice for
`proc-macro-error2 v2.0.1`; Task 4 code is warning-free under clippy with
warnings denied.

## Self-review and limitations

- No production or public test API can allow loopback/private destinations.
  Unit tests inject module-private resolver/executor/body fakes instead.
- Redirects and snapshot replays cannot reset the 30-second total deadline;
  replays also retain their hop's original three-second user-DNS deadline.
- Missing or mismatched peer data, mixed DNS sets, invalid discovery state,
  ambiguous metadata, unsupported encodings, and all limit/deadline failures
  fail closed before unsafe processing.
- The production reqwest path is exercised through private deterministic seams
  and, after the hardening wave below, a test-only loopback read-timeout server.
  The plan's opt-in public live smoke remains a later task.

## Post-review deadline, DNS, and TLS-provider hardening

The review after commit `4bd66c9` identified deadline completion bias, loss of
production reqwest body timeout classification, permissive user-DNS negative
response handling, and an ambiguous rustls provider graph. The follow-up keeps
the public transport policy unchanged while tightening these boundaries.

### RED evidence

```text
decoding_obeys_the_absolute_hop_deadline_while_in_progress
RED: fetch returned a successful 2 MiB Document after the hop clock advanced

expired_hop_request_budget_is_not_mislabeled_as_total
RED: expired hop budget returned timeout stage Total instead of Hop

reqwest_body_timeout_preserves_typed_body_idle_classification
RED: a reqwest streaming io::ErrorKind::TimedOut became generic Network

user_dns_accepts_only_noerror_nodata_as_an_empty_family
RED: NXDOMAIN NoRecordsFound returned Ok(()) instead of DnsResolveError::Lookup

decode_completion_at_exact_deadline_is_rejected
RED: a one-byte decode completing exactly at the hop deadline returned Document

body_chunk_at_exact_idle_deadline_is_rejected
RED: a chunk ready exactly at the idle deadline was accepted

production_client_builds_after_idempotent_ring_provider_installation
RED: installer and production client-builder seams did not exist; the active
     graph also reached both rustls ring (SQLx) and AWS-LC (reqwest)
```

### Hardening changes

- Every async absolute-deadline gate now performs strict pre-await and
  post-ready `now >= deadline` rejection. This covers system user DNS, RFC 7050
  discovery, NAT64 refresh-lock acquisition, fetch DNS, first byte, body
  chunk/end, and decoding. Deadline ties use Total, then Hop, then the operation
  stage.
- In-memory decoding cooperatively yields after each bounded output chunk and is
  wrapped in `min(hop,total)`, allowing ready async-compression loops to be
  preempted. Tests cover in-progress Hop/Total expiry and exact-deadline
  completion.
- Reqwest body errors preserve `is_timeout()` before URL stripping. Current
  Total/Hop/BodyIdle deadlines classify the timeout; non-timeout errors remain
  Network. A private production `ReqwestExecutor` test pins `timeout.test` to a
  test-only loopback listener, receives real headers, stalls the body, and
  verifies the real one-second reqwest read timeout becomes BodyIdle. No public
  or production address-policy bypass was added.
- Reqwest request timeout is calculated only after the pre-executor snapshot
  gate, so refresh/lock waits cannot lengthen its hop/total budget.
- User-host A/AAAA `NoRecordsFound` is accepted only for `NoError` NODATA;
  NXDOMAIN and other rcodes fail closed.
- Reqwest now uses `rustls-no-provider`; direct rustls enables only ring. The
  application installs the exact ring provider after the `--version` fast path
  and before tracing/config/database work. Installation is idempotent only for
  the complete ring provider configuration and conflicts fail hard. The
  production client builder repeats the check. AWS-LC packages are absent from
  the locked active graph.

### Final verification results

```text
cargo fmt --check
PASS

cargo clippy --locked --all-targets --all-features -- -D warnings
PASS

cargo test --locked feeds:: -- --nocapture
PASS: 48 passed, 0 failed

cargo +1.94.0 test --locked feeds:: -- --nocapture
PASS: 48 passed, 0 failed

cargo test --locked --all-features
PASS: 134 passed, 0 failed across unit/integration/doc targets

cargo tree --locked -e features -i reqwest
PASS: rustls-no-provider and stream only

cargo tree --locked -e features -i rustls
PASS: ring is the only crypto provider; reqwest supplies std/tls12

cargo tree --locked -e features -i hickory-resolver
PASS: system-config and tokio only

! cargo tree --locked -i aws-lc-rs
! cargo tree --locked -i aws-lc-sys
PASS: neither package is present

! cargo tree --locked -e features | rg \
  'system-proxy|cookies|native-tls|http2|http3'
PASS

! cargo tree --locked -e features | rg \
  'hickory-resolver feature "(dnssec|h3|https|quic|tls)[^"]*"'
PASS

cargo run --locked -- --version
PASS: raindrop 0.1.0; provider installation remains after the fast path
```

The only known verification notice remains the repository's existing
`proc-macro-error2 v2.0.1` future-incompatibility warning. The one production
adapter timeout regression intentionally takes approximately one second; all
other transport tests remain deterministic and network-independent.

### Review ledger

- Minor: there is no isolated-process regression that first installs a
  conflicting rustls provider. Rustls provider state is process-global and
  cannot be reset safely inside the ordinary unit-test process; the installer
  still compares the complete installed provider and fails closed at runtime.
- Minor: the `--version` provider-free fast path is command-verified rather
  than covered by a dedicated subprocess regression.

Neither limitation blocks Task 4: the independent final review reported zero
Critical and zero Important findings and marked the code ready.
