# RSS data Task 3 implementation report

## Status

Implemented the Task 3 safe feed primitives from
`.superpowers/sdd/rss-data-task-3-brief.md` in the requested worktree, without
adding DNS discovery, HTTP transport, parsing, persistence, HTML, AI, plugin, or
MCP behavior.

## TDD evidence

The initial `tests/feed_primitives.rs` seam imported the specified public
`raindrop::feeds` API before any feed module existed.

Initial RED command:

```text
cargo test --locked --test feed_primitives -- --nocapture
```

Result: exit 101 with `E0432`, `could not find feeds in raindrop`. The failure
was specifically the missing public module required by the brief, rather than a
test-only bypass or an unrelated dependency failure.

The implementation then proceeded through URL, address, identity, validator,
and scheduler slices. After the review-fix wave described below, the primitive
suite contains 43 public-interface behavior tests and is GREEN on both the
current toolchain and Rust 1.94.0.

## Implementation

- Added direct `http`, `httpdate`, and `ipnet` dependencies while preserving the
  exact `feedparser-rs = 0.5.5` no-default-features pin and reusing `base64`.
- Added URL normalization with raw and normalized 4,096-byte limits, pre-parse
  control/space rejection, raw-authority userinfo and empty-port checks, strict
  post-IDNA LDH validation, root-dot/default-port/fragment removal, duplicate
  query preservation, redacted formatting, and HTTPS downgrade protection.
- Added the frozen IPv4/IPv6 public-only policy, exact transition-form
  classification order, trusted `Ipv6Net` NAT64 prefix validation, RFC 6052 bit
  extraction for all six prefix lengths, u-octet rules, and typed constructor
  errors.
- Added GUID/URL/fingerprint entry identities, Unicode whitespace normalization,
  fixed v1 frames, versioned BLAKE3 derive-key hashing, exact database kind
  strings, size boundaries, and redacted formatting.
- Added opaque validator header/storage round trips with canonical `v1:` URL-safe
  unpadded base64, exact URL reuse binding, non-UTF-8 preservation, strict decode
  rejection, and sensitive `HeaderValue` propagation on construction, rebuild,
  access, and clone.
- Added receipt-anchored Retry-After parsing for delta seconds and the three
  compatible HTTP-date forms, saturating retry instants, signed persisted-count
  validation, scheduler-owned increment/reset, inclusive full jitter, four-hour
  cap, and checked UTC scheduling.

## Normative vectors and boundaries

The tests assert the brief's exact GUID and URL index frames/hashes, ordinary
fingerprint frame/fingerprint/index hash, and content-only
frame/fingerprint/index hash. They also cover:

- URL normalization/rejection/redaction, 4,096-byte limits, strict DNS label and
  host limits, insecure HTTP policy, redirects, and non-standard IPv4 text.
- Every frozen native IPv4 CIDR start/end boundary, native IPv6 boundaries,
  mapped/compatible/WKP/configured NAT64/6to4/Teredo handling, RFC 6052 vectors,
  special and overlapping prefix rejection, u octets, and suffix variation.
- GUID priority, canonical URL fallback, fingerprint degradation semantics,
  field normalization and 64 KiB limits, concatenation ambiguity resistance,
  and enclosure URL normalization.
- Validator 1/8,192/8,193-byte boundaries, non-UTF-8 bytes, canonical storage,
  exact URL matching, corrupt storage, sensitivity, and Debug redaction.
- Negative/zero/max persisted counts, first/sixth/seventh/max failure bounds,
  inclusive and invalid jitter, delta/date/past/skew/overflow Retry-After cases,
  checked next-time overflow, and the four-hour cap.

## Initial implementation verification results

All commands were run from
`/home/czyt/code/rust/raindrop/.worktrees/foundation-bootstrap` after dependency
resolution, using `--locked` where required.

```text
cargo test --locked --test feed_primitives -- --nocapture
PASS: 40 passed, 0 failed

cargo fmt --check
PASS: exit 0

cargo clippy --locked --all-targets --all-features -- -D warnings
PASS: exit 0

cargo +1.94.0 test --locked --test feed_primitives -- --nocapture
PASS: 40 passed, 0 failed

cargo test --locked --all-features
PASS: 83 passed, 0 failed across all unit/integration/doc test targets

git diff --check
PASS: exit 0
```

Cargo emitted the repository's existing future-incompatibility notice for
`proc-macro-error2 v2.0.1`; it did not produce a warning from Task 3 code or fail
clippy.

## Lockfile and scope audit

- `Cargo.lock` adds only `ipnet 2.12.0` as a package and the root direct edges for
  `http`, `httpdate`, and `ipnet`; existing locked `url 2.5.8`, `idna 1.1.0`,
  `http 1.4.2`, `httpdate 1.0.3`, and `blake3 1.8.5` remain unchanged.
- There is no direct `idna` dependency.
- `feedparser-rs` remains exactly `=0.5.5` with `default-features = false`.
- The initial implementation commit `961d0ac` staged only Task 3 files plus this
  report. Commit `e99bd97` separately committed the reviewed plan/API
  corrections after implementation; the main controller explicitly authorized
  that documentation-only scope under the user's delegated internal review
  authority. This review-fix wave does not alter the plan. The root
  `node_modules/` directory is not touched or staged.

## Self-review and limitations

- Sensitive URL paths/queries/fragments/userinfo, GUID/identity text, enclosure
  URLs, validator bytes/storage, and raw attacker input are absent from the
  relevant Debug, Display, and error chains.
- External inputs have typed failures and explicit size or numeric boundaries.
- The URL transport accessor remains `pub(crate)`; public URL access exposes only
  the hash, canonical host, scheme, and effective port.
- Validator headers cannot be retrieved without an exact normalized request URL
  match, and every returned `HeaderValue` is sensitive.
- Address-policy construction has no test-only bypass and fails closed for
  untrusted or ambiguous prefixes.
- As required, Task 3 does not implement RFC 7050/DNS64 discovery, Hickory,
  hostname resolution, HTTP execution, redirects beyond URL policy validation,
  database writes, or feed parsing. Those remain Task 4+ handoffs.

## Review fix wave

The post-implementation review identified three security/coverage gaps and one
safe simplification. They were addressed without widening Task 3 scope:

- HTTP(S) inputs now require a case-insensitive `http:`/`https:` scheme followed
  by literal `//` and a nonempty raw authority before `Url::parse`. Single slash,
  backslash, triple slash, empty authority, and empty port forms fail closed.
  Credential-looking malformed authority segments return
  `CredentialsForbidden` before generic `Invalid`; well-formed path/query `@`
  remains non-credential data.
- Validator storage rejects an encoded portion longer than 10,923 bytes before
  alphabet scanning or base64 decoding, bounding work before allocation.
- Every RFC 6052 prefix length now has independent, hard-coded IPv6 candidates
  for public embedded `8.8.8.8` and denied special `192.0.2.33`, so a constant or
  wrong bit-slice extractor cannot satisfy both assertions.
- Entry identity uses the shared URL normalizer for the only raw authority rule.
  Case-insensitive `http:`/`https:` GUIDs with credential-looking malformed
  authorities cannot downgrade to opaque; invalid non-credential URL-like GUIDs
  may remain opaque as specified.

### Review-fix TDD evidence

```text
cargo test --locked --test feed_primitives \
  feed_url_policy_requires_literal_double_slash_after_http_schemes -- --nocapture
RED: exit 101; old code accepted https:/example.com/feed

cargo test --locked --test feed_primitives \
  opaque_validator_rejects_oversized_encoded_storage_before_decoding -- --nocapture
RED: exit 101; old code returned InvalidEncoding instead of TooLong

cargo test --locked --test feed_primitives \
  feed_url_policy_requires_literal_double_slash_after_http_schemes -- --nocapture
RED priority check: exit 101; old gate returned Invalid instead of
CredentialsForbidden for https:/user@example.com/feed
```

The RFC 6052 change strengthened an already-correct implementation with
independent positive/negative literals; the new coverage was GREEN when first
run. The identity simplification was protected by credential-like malformed URL,
invalid non-credential URL-like, and well-formed path/query `@` regressions.

### Review-fix verification results

```text
cargo test --locked --test feed_primitives -- --nocapture
PASS: 43 passed, 0 failed

cargo +1.94.0 test --locked --test feed_primitives -- --nocapture
PASS: 43 passed, 0 failed

cargo fmt --check
PASS: exit 0

cargo clippy --locked --all-targets --all-features -- -D warnings
PASS: exit 0

cargo test --locked --all-features
PASS: 86 passed, 0 failed across all unit/integration/doc test targets

git diff --check
PASS: exit 0
```

The same existing `proc-macro-error2 v2.0.1` future-incompatibility notice was
emitted by Cargo; Task 3 code remained warning-free under clippy `-D warnings`.

### Review-fix self-review

- The raw authority analyzer is shared by feed URL and identity URL
  normalization; identity contains no duplicate authority scan.
- Credential error priority is fixed before parser normalization, while path and
  query `@` are inspected only after a valid nonempty raw authority boundary.
- The validator encoded-size check precedes both the alphabet walk and decode.
- RFC 6052 extraction remains private; tests exercise only `AddressPolicy`.
- `Cargo.toml` and `Cargo.lock` are unchanged by this review wave, and
  `node_modules/` remains untracked and unstaged.

## Second review fix wave

A locked `url 2.5.8` probe showed that separator variants with an empty raw
authority can still be promoted into userinfo plus a host. The shared pre-parser
authority analyzer now inspects the authority-like segment after the initial
literal `//` before returning generic `Invalid` for an empty raw authority.

The feed URL and GUID regressions cover:

- `https:///user@example.com/feed`
- `https:////user@example.com/feed`
- `https://\user@example.com/feed`
- empty-username forms `https:///@example.com/feed` and
  `https:////@example.com/feed`

All return `CredentialsForbidden` before `Url::parse`, preventing identity GUIDs
from downgrading to opaque. Matching triple/quad-slash and backslash forms
without `@` remain `Invalid`, and well-formed path/query `@` remains allowed.

### Second-wave TDD evidence

```text
cargo test --locked --test feed_primitives \
  feed_url_policy_requires_literal_double_slash_after_http_schemes -- --nocapture
RED: exit 101; old empty-authority branch returned Invalid instead of
CredentialsForbidden for https:///user@example.com/feed

cargo test --locked --test feed_primitives \
  feed_url_policy_requires_literal_double_slash_after_http_schemes -- --nocapture
GREEN: 1 passed, 0 failed

cargo test --locked --test feed_primitives \
  entry_identity_rejects_credential_like_url_guids_without_leaking_them -- --nocapture
GREEN: 1 passed, 0 failed
```

The empty-username vectors were added after the core fix and passed immediately,
confirming that the same shared analyzer covers both explicit and promoted empty
userinfo markers.

### Second-wave verification results

```text
cargo test --locked --test feed_primitives -- --nocapture
PASS: 43 passed, 0 failed

cargo +1.94.0 test --locked --test feed_primitives -- --nocapture
PASS: 43 passed, 0 failed

cargo fmt --check
PASS: exit 0

cargo clippy --locked --all-targets --all-features -- -D warnings
PASS: exit 0

cargo test --locked --all-features
PASS: 86 passed, 0 failed across all unit/integration/doc test targets

git diff --check
PASS: exit 0
```

Self-review confirmed that the change remains confined to the shared raw HTTP
authority analyzer and public-seam regressions. It does not expose parser
internals, alter valid URL normalization, change dependencies, or touch
`node_modules/`.
