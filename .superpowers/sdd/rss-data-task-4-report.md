# RSS data Task 4 implementation report

## Status

Implemented the pinned, bounded feed HTTP transport from the regenerated Task 4
brief and preflight, using `672f03d` as the implementation/review base. The
change is limited to the authorized dependency files, `src/feeds` transport
files, and this report. The untracked root `node_modules/` directory was not
touched.

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

## Verification

All commands were run from
`/home/czyt/code/rust/raindrop/.worktrees/foundation-bootstrap`.

```text
cargo tree --locked -e features -i reqwest
PASS: only reqwest rustls/stream (and internal rustls support features)

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
- The production reqwest path is configured and type-checked without a live
  network test in this task. Deterministic behavior is exercised through the
  private executor seam; the plan's opt-in public live smoke remains a later
  task.
