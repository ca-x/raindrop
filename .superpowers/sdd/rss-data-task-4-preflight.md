# RSS data Task 4 API/security preflight

Read this together with `rss-data-task-4-brief.md`. These are compile- and
security-critical facts verified against the exact pinned crate sources.

## Hickory 0.26.1

- `ResolverOpts` is `#[non_exhaustive]`. Build the system resolver with
  `TokioResolver::builder_tokio()?`, then mutate `builder.options_mut()` and
  call `build()`. Do not construct `ResolverOpts` with a struct literal and do
  not replace system options with `ResolverOpts::default()`.
- The dedicated RFC 7050 resolver sets `use_hosts_file=ResolveHosts::Never`,
  `cache_size=0`, `attempts=1`, and all positive/negative TTL min/max fields to
  `None`. `attempts=1` still permits one retry after the initial wire request.
- Run explicit A and AAAA lookups concurrently under one shared timeout with
  `tokio::join!`, not `try_join!`: valid AAAA `NOERROR/NODATA` is returned as
  `Err(NetError::Dns(DnsError::NoRecordsFound(no_records)))`.
- Error imports are `hickory_resolver::net::{NetError, DnsError}` and response
  codes are `hickory_resolver::proto::op::ResponseCode`. These enums and
  `NoRecords` are non-exhaustive; matches need fallback arms / `..`.
- Accept absent AAAA only when `response_code == NoError` and
  `negative_ttl == Some(ttl)` with `ttl > 0`; checked-add the TTL to the same
  monotonic clock. NXDOMAIN, missing/zero TTL, overflow, and every other error
  fail closed.
- `Lookup::valid_until()` is `std::time::Instant`, not Tokio Instant. Keep the
  clock types explicit and convert only at deadline boundaries. Tokio paused
  time does not advance Hickory's real `std::Instant`; fake TTL tests need an
  injected/fake clock or fake deadlines.
- `Lookup` has no public `iter()`. Traverse `answers()` and match record data
  `RData::A(A(ip))` / `RData::AAAA(AAAA(ip))`.
- Hickory queries use absolute FQDN names with a trailing root dot so system
  search domains are not applied.
- User-host A/AAAA absence is accepted only for `NoRecordsFound` with
  `response_code == NoError`; NXDOMAIN and all other rcodes fail closed rather
  than becoming an empty address family.
- Require verified A records containing both WKAs for both Present and
  NotPresent. Present validity is `min(A deadline, AAAA deadline)`; NotPresent
  validity is `min(A deadline, checked now + negative AAAA TTL)`. `now >=
  valid_until` is expired; refresh is on-demand single-flight before user DNS.

## DNS pinning and reqwest 0.13.4

- Hickory may query `example.com.`, but `ClientBuilder::resolve_to_addrs` must
  use the exact root-dot-free `NormalizedFeedUrl::canonical_host()` key. A
  dotted override key misses and falls back to ambient DNS.
- Approved socket addresses use `effective_port()`. Validate the full
  `SocketAddr` from `Response::remote_addr()` before reading redirect Location
  or consuming/moving the response. Missing or mismatched peer fails closed.
- IP-literal hosts are classified and pinned directly without user-host DNS.
- Strip URLs immediately with `reqwest::Error::without_url(self)` before any
  `?`, `#[from]`, anyhow, or custom wrapping. Reqwest Debug and Display include
  attached URLs otherwise.
- Per hop, call `no_proxy`, every `no_*` decompression method, redirect none,
  connect timeout, read timeout, and a total timeout equal to the smaller of
  remaining hop and refresh time. Expired deadlines are rejected before
  starting work.
- The 20-second deadline is per HTTP request/redirect hop and resets only for a
  new redirect target. DNS/snapshot replays inside that hop retain it. The
  30-second refresh deadline never resets.
- Reqwest `read_timeout` resets on ready frames, not strictly only data chunks.
  Wrap each body `chunk()`/stream-next future in an external absolute
  `min(body_idle, hop, total)` deadline and refresh body idle only after a
  non-empty data chunk.
- Preserve reqwest body `is_timeout()` before stripping its URL. Classify that
  timeout using current deadline priority Total, then Hop, then BodyIdle;
  non-timeout body errors remain typed network failures.
- Do not call `Response::bytes()` or `read_to_end` before limits. Checked-add
  compressed chunks before append; reject before exceeding 2 MiB.

## Decoding and redirect details

- HTTP `deflate` is zlib-wrapped: use
  `async_compression::tokio::bufread::ZlibDecoder`. Do not enable the `deflate`
  feature or use `DeflateDecoder`, which accepts raw DEFLATE.
- `GzipDecoder`, `BrotliDecoder`, and `ZlibDecoder` require `AsyncBufRead`.
  Tokio implements it for `std::io::Cursor<Vec<u8>>`; no undeclared
  `tokio-util` dependency is needed. Enable gzip `multiple_members(true)` so
  later members cannot bypass decoded/ratio accounting.
- Decode with a fixed-size buffer and enforce 10 MiB plus the checked
  `compressed_len * 100` ratio during streaming, before append.
- Cooperatively yield between bounded decoded output chunks. Wrap decoding in
  the absolute `min(hop,total)` deadline and reject `now >= deadline` both
  before awaiting and after the decoder reports ready.
- Resolve relative Location with the current complete URL's `Url::join`, then
  pass the absolute result through `normalize_redirect`. Do not pass a relative
  Location directly to Task 3 normalization.
- Reuse validators only through `ValidatorSet::for_request(&url)`; changed path
  or query receives none.
- Document outcomes preserve final URL, decoded bytes, exactly one optional
  UTF-8 Content-Type string, and optional ETag/Last-Modified opaque validators.
  304 preserves final URL and optional response validators without body polls.
  Multiple metadata fields or non-UTF-8 Content-Type fail typed and Debug shows
  only counts/presence.
- Accept Content-Encoding only when absent or exactly one OWS-trimmed,
  case-insensitive token `identity`, `gzip`, `br`, or `deflate`. Reject empty,
  multiple fields, comma lists, parameters, non-ASCII and unknown tokens.
- Address count is checked on the raw resolver vector (1..=16), then exact
  duplicates are removed. Allow five redirect responses and reject the sixth.
  Ratio equality at 100:1 is accepted; the next decoded byte rejects.
- Status handling: 200 document, 304 without body polling, only
  301/302/303/307/308 redirects with exactly one valid Location, all others a
  typed status error without body polling. Preserve one valid Retry-After using
  response receipt time; multiple/invalid values are typed response errors.

## Dependency feature gate

- Reqwest stays
  `default-features=false, features=["rustls-no-provider","stream"]`.
- Add direct `rustls 0.23.42` with default features disabled and only `ring`.
  Install the exact ring provider immediately after the binary `--version`
  fast path, and idempotently before production reqwest client build. A
  conflicting process-global provider is a startup/configuration error.
- Hickory stays `default-features=false, features=["system-config","tokio"]`.
- Tokio production adds only `sync,time`; `test-util` is dev-only.
- No system proxy, cookies, automatic codecs, native TLS, HTTP/2, HTTP/3,
  Hickory DNSSEC, DoT, DoH, DoQ, or DoH3 features.
- Neither `aws-lc-rs` nor `aws-lc-sys` may be reachable in `Cargo.lock`'s active
  dependency graph.
