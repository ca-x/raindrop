# Raindrop RSS Data Ingestion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the trustworthy RSS record path from a subscription URL to shared Feed/Entry rows, including portable schema, secure network fetching, parsing, HTML sanitization, fenced refresh persistence, deterministic fixtures, and an opt-in IT Home live smoke.

**Architecture:** The relational database remains the record system. Feeds and entries are shared globally; subscriptions and sparse entry state are user-owned. Network, decompression, parsing, sanitization, and synchronous content processing happen outside database transactions. Short transactions use unique constraints, monotonic feed sequences, a global ingest generation, and lease fencing tokens for idempotent persistence.

**Tech Stack:** Rust 1.94, Axum 0.8, Tokio 1.52, SeaORM/sea-orm-migration declared as 1.1.19 and locked to 1.1.20, reqwest 0.13.4 with rustls and stream only, feed-rs 2.4.0 without `sanitize`, quick-xml 0.41.0, ammonia 4.1.3, async-compression 0.4.42, ipnet 2.12.0, BLAKE3, SQLite/PostgreSQL/MySQL.

## Global Constraints

- Rust edition 2024 and MSRV 1.94; committed `Cargo.lock` is authoritative and all Cargo verification uses `--locked` after dependency resolution.
- Feed and Entry are shared records. Subscription and EntryState are always scoped by an authenticated `user_id`.
- Use `ingest_generation + feed_sequence`, not timestamps, for stable list and mark-read snapshots.
- Every refresh lease has a monotonically increasing `lease_token`; a worker that loses owner/token fencing cannot commit.
- Feed identity and entry identity use BLAKE3 hash indexes plus full normalized-value comparison; hash collision is an explicit error, never a silent merge.
- Do not execute network, XML/JSON parsing, HTML sanitization, plugins, AI, MCP, or notifications inside a database transaction.
- Default subscription policy accepts HTTPS only. HTTP requires an instance-level `allow_insecure_http` policy; HTTPS redirects may never downgrade to HTTP.
- Disable automatic redirects, ambient/system proxy use, and automatic response decompression. Revalidate and pin DNS addresses on every redirect hop, with at most five redirects.
- Default limits are DNS 3 s, connect/TLS 5 s, first byte 10 s, body idle 10 s, one hop 20 s, whole refresh 30 s, compressed body 2 MiB, decoded body 10 MiB, compression ratio 100:1, 5,000 entries, and XML depth 128.
- Server-side HTML sanitization is mandatory. React never receives raw Feed XML or unsanitized publisher HTML; remote images remain inert by default.
- Deterministic fixtures and local scripted servers are mandatory CI evidence. `https://www.ithome.com/rss/` is an ignored, opt-in smoke guarded by `RAINDROP_LIVE_RSS_SMOKE=1` and is not a pull-request gate.
- API and module interfaces use typed inputs/outputs and explicit error enums. Route handlers do not fetch URLs, parse feeds, sanitize HTML, or issue database queries.
- New dependencies must have minimal features, no system proxy, no cookie jar, no native-tls, no HTTP/3, no unused codecs, and no high/critical reachable advisory.

---

### Task 1: Portable RSS record schema and entities

**Files:**

- Modify: `src/db/entities.rs`
- Create: `src/db/entities/rss_counter.rs`
- Create: `src/db/entities/feed.rs`
- Create: `src/db/entities/subscription.rs`
- Create: `src/db/entities/entry.rs`
- Create: `src/db/entities/entry_state.rs`
- Modify: `src/db/migration.rs`
- Create: `src/db/migration/rss/mod.rs`
- Create: `src/db/migration/rss/counters.rs`
- Create: `src/db/migration/rss/feeds.rs`
- Create: `src/db/migration/rss/subscriptions.rs`
- Create: `src/db/migration/rss/entries.rs`
- Create: `src/db/migration/rss/entry_states.rs`
- Create: `tests/rss_migrations.rs`

**Interfaces:**

- Produces SeaORM entities `rss_counter`, `feed`, `subscription`, `entry`, and `entry_state` under `crate::db::entities`.
- Preserves the existing identity migration as migration 1 and appends five ordered RSS migrations.
- `rss_counters`: `key VARCHAR(32)` primary key, `value BIGINT NOT NULL`; migration inserts `INGEST_GENERATION=0` idempotently.
- `feeds`: `id VARCHAR(36)`; `source_url/normalized_url/fetch_url TEXT`; `normalized_url_hash VARCHAR(64)`; nullable `validator_url/etag/last_modified TEXT`; nullable `response_content_hash VARCHAR(64)`; `entry_sequence_head BIGINT`; nullable `last_attempt_at/last_success_at/last_changed_at`; non-null `next_fetch_at`; nullable `retry_after_at`; `consecutive_failures BIGINT`; nullable `last_error_code VARCHAR(64)`; `is_disabled`; nullable `orphaned_at/lease_owner/lease_until`; monotonic `lease_token BIGINT`; non-null `created_at/updated_at`.
- `subscriptions`: UUID-string `id`, `user_id`, `feed_id`, display override/position, `start_sequence`, `read_through_sequence`, `state_revision`, created/updated timestamps. Unique `(user_id, feed_id)`.
- `entries`: `id/feed_id VARCHAR(36)`; immutable `feed_sequence/ingest_generation BIGINT`; `identity_kind VARCHAR(16)`, full `identity TEXT`, `identity_hash VARCHAR(64)`; nullable canonical URL/title/author/summary; non-null `sanitized_content TEXT`; nullable untrusted UTC Unix-microsecond `published_at_us BIGINT`; immutable `sort_at_us BIGINT`; non-null inserted/updated operational timestamps; `source_content_hash/content_hash VARCHAR(64)`; `pipeline_version VARCHAR(64)`; nullable `direction VARCHAR(8)` and nullable versioned `enclosure_json TEXT`.
- `entry_states`: primary key `(user_id, entry_id)` plus redundant immutable `feed_id/feed_sequence`, nullable `read_override`, starred fields, revision, updated time. Composite foreign keys require an owned subscription and the matching entry tuple.
- All hashes are lower-case 64-character BLAKE3 hex. IDs are lower-case UUID strings of length 36. JSON remains versioned `TEXT`; no native DB enums/UUID/JSON or partial indexes.

- [ ] **Step 1: Write the failing SQLite migration contract**

Add these exact tests using fixed UUIDs, fixed UTC timestamps, and SeaORM ActiveModels:

- `sqlite_rss_migrations_are_idempotent_and_seed_generation`: create temporary SQLite, run `migrate` twice, query `rss_counters[INGEST_GENERATION] == 0`, and assert `feeds`, `subscriptions`, `entries`, and `entry_states` accept a valid ownership chain.
- `rss_schema_shares_feeds_but_rejects_duplicate_user_subscription`: insert two users and one Feed, insert one Subscription for each user, then assert a second `(same user_id, same feed_id)` insert returns a database error while the two-user shared Feed remains one row.
- `rss_state_foreign_keys_reject_cross_user_and_mismatched_entry_rows`: insert one Entry and only user A's Subscription; assert user B's EntryState fails, then insert user B's Subscription and assert an EntryState with the wrong `feed_sequence` fails.
- `deleting_a_user_cascades_subscription_and_state_without_deleting_shared_feed_entries`: delete user A and assert A's Subscription/EntryState are gone while the shared Feed, Entry, user B Subscription, and user B EntryState remain.

Verify named unique constraints through behavior, not SQLite catalog string matching.

- [ ] **Step 2: Run the contract and verify it fails before RSS tables exist**

Run:

```bash
cargo test --locked --test rss_migrations -- --nocapture
```

Expected: FAIL because RSS entities/migrations do not exist.

- [ ] **Step 3: Add focused entities and ordered migrations**

`src/db/entities.rs` keeps the existing inline identity entities and adds:

```rust
pub mod entry;
pub mod entry_state;
pub mod feed;
pub mod rss_counter;
pub mod subscription;
```

Each new entity uses explicit string IDs, `i64` for BIGINT counters/sequences, `OffsetDateTime` for timestamps, `Option<bool>` for `read_override`, and `String` for versioned JSON/enums.

`Migrator::migrations()` becomes:

```rust
vec![
    Box::new(CreateIdentityTables),
    Box::new(rss::counters::CreateRssCounters),
    Box::new(rss::feeds::CreateFeeds),
    Box::new(rss::subscriptions::CreateSubscriptions),
    Box::new(rss::entries::CreateEntries),
    Box::new(rss::entry_states::CreateEntryStates),
]
```

Also add `mod rss;` to `src/db/migration.rs`. SQLite foreign keys are defined inline in each `CREATE TABLE`. Standalone indexes use `SchemaManager::create_index`, and every MySQL-reentrant index path checks `SchemaManager::has_index` before creation.

Use explicitly named constraints/indexes:

```text
uq_feeds_url_hash(normalized_url_hash)
idx_feeds_due(is_disabled,next_fetch_at,lease_until,id)
uq_subscriptions_user_feed(user_id,feed_id)
idx_subscriptions_user_pos(user_id,position,id)
idx_subscriptions_feed(feed_id,id)
uq_entries_feed_identity(feed_id,identity_hash)
uq_entries_feed_seq(feed_id,feed_sequence)
uq_entries_state_tuple(id,feed_id,feed_sequence)
idx_entries_feed_list(feed_id,sort_at_us,id)
idx_entries_all_list(sort_at_us,id,feed_id)
idx_entries_snapshot(feed_id,ingest_generation,feed_sequence)
idx_states_feed_read(user_id,feed_id,read_override,feed_sequence)
idx_states_starred(user_id,is_starred,starred_at,entry_id)
```

Foreign-key delete actions are exact: users → subscriptions CASCADE; feeds → subscriptions RESTRICT; feeds → entries CASCADE; `(user_id,feed_id)` subscriptions → entry_states CASCADE; `(entry_id,feed_id,feed_sequence)` entries → entry_states CASCADE.

Down migrations drop in strict reverse dependency order. Application code remains responsible for non-negative sequences and `read_through_sequence >= start_sequence`; portable CHECK behavior is not the only safety boundary.

- [ ] **Step 4: Run migration and existing database tests**

Run:

```bash
cargo test --locked --test rss_migrations -- --nocapture
cargo test --locked --test database_migrations -- --nocapture
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Expected: PASS; existing identity migration behavior remains unchanged.

- [ ] **Step 5: Commit**

```bash
git add src/db tests/rss_migrations.rs
git commit -m "feat: add rss record schema"
```

---

### Task 2: Cross-database schema, time, and migration-reentry contract

**Files:**

- Modify: `src/db/connect.rs`
- Modify: `tests/rss_migrations.rs`
- Create: `tests/support/mod.rs`
- Create: `tests/support/database.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/configuration.md`

**Interfaces:**

- One `rss_schema_contract(database_url)` test body runs against mandatory temporary SQLite and opt-in `RAINDROP_TEST_POSTGRES_URL` / `RAINDROP_TEST_MYSQL_URL` connections.
- CI provides PostgreSQL and MySQL services and runs the same contract for both; local developers may omit those environment variables.
- Connection initialization configures every future pool connection at handshake: `map_sqlx_postgres_opts(|opts| opts.options([("timezone", "UTC")]))` and `map_sqlx_mysql_opts(|opts| opts.timezone(Some("+00:00".to_owned())))`. Tests acquire more than one pool connection and verify UTC/microsecond round trips for operational `OffsetDateTime` fields.
- Untrusted source dates remain `published_at_us: Option<i64>` and never enter MySQL `TIMESTAMP`. Parsing rejects values outside signed Unix-microsecond representation; display conversion is outside SQL.
- MySQL migration reentry is proven by a partial-state contract: precreate a target table/index state, rerun the corresponding migration path, and assert all expected named indexes/seed rows exist without broad `INSERT IGNORE`.
- `INGEST_GENERATION` seed uses an exact primary-key conflict path and validates an existing row's value/type; unrelated database errors remain visible.

- [ ] **Step 1: Extract a backend-parameterized schema contract**

Move fixed user/feed/subscription/entry/state setup into `tests/support/database.rs`. The public test seam is `db::migrate` plus SeaORM entity/constraint behavior. SQLite always runs; PostgreSQL/MySQL tests are marked skipped only when their environment URL is absent and print no credentials.

- [ ] **Step 2: Add failing UTC/range/reentry cases**

Add exact cases for operational timestamp UTC microsecond roundtrip, publisher dates before 1970 and after 2038 through `published_at_us`, MySQL case-insensitive collation not merging different full identities after hash lookup, and partial index/seed migration recovery.

- [ ] **Step 3: Configure backend sessions and portable migration reentry**

Configure PostgreSQL/MySQL connect options before pool creation so every newly opened connection receives UTC settings; a one-time post-connect `SET` is insufficient. Never include a database URL in logs/errors. Update migration helpers so standalone named indexes call `has_index` before creation where required and the counter seed handles only the exact primary-key conflict.

- [ ] **Step 4: Add CI services and run all three contracts**

Run locally for SQLite, then in CI with PostgreSQL/MySQL service URLs:

```bash
cargo test --locked --test rss_migrations -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test rss_migrations postgres -- --nocapture
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test rss_migrations mysql -- --nocapture
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Expected: the same contract passes on all three backends without printing connection secrets.

- [ ] **Step 5: Commit**

```bash
git add src/db/connect.rs tests/rss_migrations.rs tests/support .github/workflows/ci.yml docs/configuration.md
git commit -m "test: verify rss schema across databases"
```

---

### Task 3: URL, address, validator, entry identity, and scheduling primitives

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/feeds/mod.rs`
- Create: `src/feeds/error.rs`
- Create: `src/feeds/model.rs`
- Create: `src/feeds/url_policy.rs`
- Create: `src/feeds/address_policy.rs`
- Create: `src/feeds/identity.rs`
- Create: `src/feeds/schedule.rs`
- Modify: `src/lib.rs`
- Create: `tests/feed_primitives.rs`

**Interfaces:**

- `FeedUrlPolicy::new(allow_insecure_http: bool)` validates and normalizes absolute URLs up to 4,096 bytes.
- `NormalizedFeedUrl` contains the full normalized URL, BLAKE3 hash, canonical host, scheme, and effective port; it never implements `Display` with query-bearing output.
- `AddressPolicy::public_only()` returns `Allowed` only for globally routable unicast addresses and unwraps IPv4-mapped/transition forms before classification.
- `EntryIdentity::from_parts(guid, canonical_url, stable_fields)` uses `GUID -> URL -> FINGERPRINT`; fetch time is never part of a fingerprint.
- `ValidatorSet` is only reusable when its exact `validator_url` matches the request URL; validators are opaque header values.
- `RefreshSchedule<J: JitterSource>::after_result(now, result, failures, retry_after: Option<RetryAfter>)` implements a 5-minute base, injected deterministic full jitter, and a 4-hour maximum.
- `RetryAfter` is an already parsed absolute UTC instant. Parsing accepts delta-seconds and IMF-fixdate via `httpdate`; past dates become zero delay. The calculation is `min(max(jittered_backoff, retry_after_delay), 4 hours)`.

- [ ] **Step 1: Write failing table-driven primitive tests**

Cover:

```text
HTTPS normalization: host case, IDNA, root dot, default port, empty path, fragment removal, duplicate query preservation
Rejection: credentials, controls, oversized input, relative/network-path URL, unsupported scheme, malformed port
HTTP matrix: default reject; explicit insecure policy accepts HTTP; HTTPS-to-HTTP redirect rejects
Addresses: private, loopback, link-local, CGNAT, metadata, documentation, multicast, unspecified, reserved, IPv4-mapped IPv6, NAT64/6to4/Teredo embedding denied IPv4
Identity: opaque GUID, URL GUID normalization, canonical URL fallback, deterministic fingerprint fallback, fetch-time independence
Validators: same exact URL allowed; changed final URL or origin does not reuse
Schedule: success/304 reset; transient exponential backoff with deterministic jitter; Retry-After delta/date/past/skew cases bounded to 4 hours
```

- [ ] **Step 2: Run the tests and verify missing modules fail**

```bash
cargo test --locked --test feed_primitives -- --nocapture
```

Expected: FAIL because `raindrop::feeds` does not exist.

- [ ] **Step 3: Implement immutable primitives with explicit errors**

Errors use variants, not string parsing:

```rust
pub enum FeedUrlError {
    Empty,
    TooLong,
    ControlCharacter,
    Invalid,
    UnsupportedScheme,
    InsecureHttpDisabled,
    CredentialsForbidden,
    MissingHost,
    HttpsDowngrade,
}

pub enum AddressDecision {
    Allowed,
    Denied(AddressDenyReason),
}
```

Add `ipnet = "2.12.0"` and `httpdate = "1.0.3"`. Normalization removes fragments/default ports/root dot, preserves query ordering, rejects credentials, and hashes the complete normalized URL. URL-bearing types use custom redacted `Debug` and do not expose query/token text. Address ranges are a fixed, reviewed CIDR table backed by `ipnet`; tests name every denied class. Fingerprints use a domain-separated BLAKE3 hasher and normalized stable text.

- [ ] **Step 4: Verify primitives**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_primitives -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/feeds src/lib.rs tests/feed_primitives.rs
git commit -m "feat: define safe feed primitives"
```

---

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

- Add `reqwest = { version = "0.13.4", default-features = false, features = ["rustls", "stream"] }`.
- Add `async-trait = "0.1.89"`, `async-compression = { version = "0.4.42", default-features = false, features = ["tokio", "gzip", "zlib", "brotli"] }`, and `futures-util = "0.3.32"`. Add Tokio `time`; add Tokio `test-util` for unit tests without changing production behavior.
- `#[async_trait::async_trait] trait DnsResolver: Send + Sync` returns all A/AAAA `IpAddr` values under a 3-second deadline. Production uses Tokio lookup; tests inject a fake resolver.
- Public `#[async_trait::async_trait] trait FeedTransport: Send + Sync { async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError>; }` is the injection seam used by later domain/integration tests. Production `HttpFeedTransport` owns private async `DnsResolver` and per-hop async `HttpExecutor` seams; unit tests inject fakes without exposing a runtime loopback bypass.
- Each hop builds a short-lived reqwest client with `redirect(Policy::none())`, `no_proxy()`, `no_gzip()`, `no_brotli()`, `no_deflate()`, `no_zstd()`, 5-second connect timeout, 10-second read-idle timeout, remaining hop/total deadlines, and `resolve_to_addrs(host, approved_socket_addrs)`.
- At most 16 DNS results and five redirects. Any denied address rejects the whole set. Every redirect repeats URL/DNS/address validation and never forwards validators across a changed validator URL.
- A successful response must have `remote_addr()` in the approved address set. Missing/mismatched peer information rejects before body processing.
- Absolute deadlines cover DNS 3 s, connect 5 s, first byte 10 s, each body-idle interval 10 s, one hop 20 s, and the whole refresh 30 s. Redirects never reset the total deadline.
- Automatic decompression is disabled. Compressed bytes are capped at 2 MiB; decoded bytes at 10 MiB; ratio at 100:1; unknown or layered encodings reject. HTTP `deflate` means zlib-wrapped DEFLATE in v1; raw DEFLATE returns a typed decode error.
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
compressed_and_decoded_limits_stop_streaming()
brotli_gzip_and_deflate_decode_within_budget()
timeouts_share_one_total_refresh_deadline()
first_byte_timeout_is_distinct_from_body_idle_timeout()
body_idle_timeout_resets_only_after_a_chunk()
redirects_do_not_reset_hop_or_total_deadlines()
connected_peer_must_match_the_approved_set()
reqwest_errors_do_not_expose_url_queries_in_debug_or_display()
```

- [ ] **Step 2: Add the smallest compiling transport/error scaffold**

Define `FeedTransport`, `FetchRequest`, `FetchOutcome`, typed errors, private resolver/executor traits, and an `HttpFeedTransport` constructor. The first behavior test must compile and fail on an assertion, not on a missing module or symbol.

- [ ] **Step 3: Implement one RED/GREEN behavior slice at a time**

Implement in this order, running the named unit test after each slice: DNS classify/pin; one-hop 200/304; peer verification; manual 301/302/303/307/308; validator scoping; first-byte/body-idle/hop/total deadlines; raw byte cap; gzip; Brotli; zlib-wrapped deflate; ratio/decoded cap; error redaction. Do not enable reqwest `default`, `system-proxy`, `cookies`, compression, native TLS, HTTP/2, or HTTP/3 features. `Location` may be relative and is resolved against the current URL before full revalidation.

Decoded bodies remain bytes; do not create an unbounded `String`. Invalid header values are not reflected in errors. Diagnostics contain feed ID/host/status/counts only, never full URL query, validator, or body.

- [ ] **Step 4: Audit dependency features and run transport tests**

```bash
cargo tree --locked -e features -i reqwest
! cargo tree --locked -e features | rg 'system-proxy|cookies|native-tls|http3'
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked feeds:: -- --nocapture
```

Expected: PASS; tree shows rustls/stream and only Brotli/gzip/zlib codecs, without system proxy, cookie jar, native-tls, or HTTP/3.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/feeds
git commit -m "feat: fetch feeds through pinned transport"
```

---

### Task 5: Feed parsing, XML preflight, and HTML sanitization

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/feeds/parse.rs`
- Create: `src/content/mod.rs`
- Create: `src/content/sanitize.rs`
- Modify: `src/lib.rs`
- Create: `tests/fixtures/rss_2_60_items.xml`
- Create: `tests/fixtures/atom_mixed.xml`
- Create: `tests/fixtures/rss_identity_edges.xml`
- Create: `tests/fixtures/malicious_html.xml`
- Create: `tests/fixtures/unsafe_xml.xml`
- Create: `tests/fixtures/feed_manifest.json`
- Create: `tests/feed_parse_sanitize.rs`

**Interfaces:**

- Add `feed-rs = { version = "2.4.0", default-features = false }`, `quick-xml = "0.41.0"`, and `ammonia = "4.1.3"`.
- `preflight_xml(&[u8])` rejects DTD/ENTITY, nesting deeper than 128, more than 1,000,000 XML events, more than 256 attributes on one element, and any single attribute value over 64 KiB before feed-rs parses.
- `parse_feed(FetchedDocument) -> Result<ParsedFeed, FeedParseError>` supports RSS/Atom/JSON Feed, caps 5,000 entries, title at 64 KiB, one entry content at 1 MiB, all normalized entry text at 16 MiB, 64 enclosures per entry, and canonical enclosure JSON at 256 KiB. It retains `published_at_us=None` when the source has no date.
- `sanitize_entry_html(base_url, input) -> SanitizedContent` removes scripts/styles/forms/frames/SVG/MathML, event handlers, style/class/id/data attributes, unsafe schemes, `srcset`, and tracking attributes.
- Anchors retain only safe HTTP(S) hrefs. Images become inert records that retain safe original URL metadata and alt/width/height but no active remote `src`.
- Content hash is calculated after sanitization. Publisher-only tracking/style changes do not create a content update.

- [ ] **Step 1: Add deterministic fixtures and failing golden tests**

The 60-item fixture is synthetic, not a copy of current IT Home content. It contains fixed timestamps, unique GUIDs, escaped HTML descriptions, relative/absolute links, remote images, and tracking attributes. Manifest fields are `format`, `expectedEntries`, `expectedIdentityHashes`, `expectedDecodedHash`, `requiredSanitizedSnippets`, and `forbiddenSanitizedSnippets`.

Required tests:

```text
RSS 2.0 fixture parses exactly 60 stable identities
Atom xml:base and published/updated map correctly
duplicate/missing GUIDs follow deterministic identity precedence
DOCTYPE/entity/deep XML reject without panic
wrong-but-tolerated MIME parses only after safe body sniff
malicious HTML loses scripts/events/styles/forms/frames/SVG/unsafe URLs
remote image HTML is inert and cannot initiate a request
tracking-only changes produce the same sanitized content hash
single-title, single-content, total-normalized-content, attribute, event, enclosure-count, and enclosure-JSON budgets reject with typed errors
```

- [ ] **Step 2: Add compiling parser/sanitizer interfaces**

Define `FetchedDocument`, `ParsedFeed`, `ParsedEntry`, `SanitizedContent`, and typed preflight/parse/sanitize errors so the first golden test compiles and fails on behavior.

- [ ] **Step 3: Implement one corpus behavior at a time**

Implement and run focused RED/GREEN tests in this order: MIME sniff; XML DTD/entity/depth/event/attribute budgets; RSS 2.0 mapping; Atom/xml:base; identity folding; per-field/total/enclosure budgets; sanitizer allowlist; inert images; stable sanitized hash. MIME handling accepts XML/JSON Feed MIME directly. `text/plain`, `text/html`, and `application/octet-stream` parse only when BOM/whitespace-stripped body sniff begins with XML/RSS/Atom/RDF or a JSON object. Binary/image/audio/video/PDF bodies reject.

Feed mapping resolves relative links against `xml:base` or final response URL, then restricts them to credential-free HTTP(S) and removes fragments. Duplicate identities within one document choose the newer source timestamp, then stable document order.

- [ ] **Step 4: Verify corpus and dependency graph**

```bash
cargo tree --locked -e features | rg "feed-rs|quick-xml|ammonia"
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_parse_sanitize -- --nocapture
```

Expected: PASS; feed-rs `sanitize` feature is absent.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/content src/feeds src/lib.rs tests/fixtures tests/feed_parse_sanitize.rs
git commit -m "feat: parse and sanitize feed content"
```

---

### Task 6: Refresh-run schema and database-clock lease fencing

**Files:**

- Create: `src/db/entities/feed_refresh_run.rs`
- Modify: `src/db/entities.rs`
- Create: `src/db/migration/rss/refresh_runs.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Create: `src/feeds/repository.rs`
- Create: `src/feeds/refresh.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_refresh_claims.rs`

**Interfaces:**

- `feed_refresh_runs` exact columns: `id VARCHAR(36) PK`; `feed_id VARCHAR(36) NOT NULL` FK feeds ON DELETE CASCADE; nullable `requested_by_user_id VARCHAR(36)` FK users ON DELETE SET NULL; `trigger_kind/status VARCHAR(32) NOT NULL`; `idempotency_key VARCHAR(64) NOT NULL`; nullable `lease_token BIGINT`; nullable `commit_generation BIGINT`; `queued_at` non-null operational timestamp; nullable `started_at/fetched_at/persisted_at/completed_at`; nullable `http_status INTEGER`; `new_count/updated_count/dropped_count INTEGER NOT NULL DEFAULT 0`; nullable `error_code VARCHAR(64)` and `retry_at`.
- Status values are `QUEUED/RUNNING/SUCCESS/NOT_MODIFIED/PARTIAL/ERROR/LEASE_LOST/CANCELLED`; trigger values are `SCHEDULED/MANUAL/SUBSCRIBE/IMPORT/RETRY`.
- Named indexes are `uq_refresh_runs_idem`, `uq_refresh_runs_generation`, `idx_refresh_runs_feed(feed_id,queued_at,id)`, and `idx_refresh_runs_status(status,queued_at,id)`.
- `FeedRepository::claim_due` performs one backend-specific conditional UPDATE, increments `feeds.lease_token`, and succeeds only when one row is affected. `RefreshClaim.lease_deadline` is diagnostic; authorization always rechecks owner/token and the live statement clock in the same UPDATE.
- Lease conditions and assignments embed live database-clock expressions directly: SQLite compares `julianday(lease_until)` with `julianday('now')` and derives the new UTC value from `strftime`; PostgreSQL uses `clock_timestamp()` plus a bound interval; MySQL uses `UTC_TIMESTAMP(6)` plus `TIMESTAMPADD`. Do not SELECT a time snapshot before lock acquisition and do not use PostgreSQL transaction-start `CURRENT_TIMESTAMP`. All extend/success/304/failure paths use the same direct expression strategy.

- [ ] **Step 1: Add the refresh-run entity/migration and a compiling repository contract**

Create typed `RefreshTrigger`, `RefreshStatus`, `RefreshClaim`, `ClaimRequest`, and `RefreshRepositoryError`. The first test compiles and fails because claim returns no row.

- [ ] **Step 2: Implement behavioral slices**

Run separate RED/GREEN cycles for: one claimant wins; second concurrent claimant loses; a newer token fences an old worker; a lock wait that crosses `lease_until` cannot commit using a pre-wait time; app clocks shifted forward/backward do not affect authorization; manual idempotency key returns the existing run; success/304/failure transitions reject invalid prior states.

- [ ] **Step 3: Verify all backends through the Task 2 harness**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_refresh_claims -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test feed_refresh_claims postgres -- --nocapture
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test feed_refresh_claims mysql -- --nocapture
cargo test --locked --test rss_migrations -- --nocapture
```

- [ ] **Step 4: Commit**

```bash
git add src/db src/feeds tests/feed_refresh_claims.rs tests/rss_migrations.rs
git commit -m "feat: fence feed refresh claims"
```

---

### Task 7: Idempotent entry persistence and ingest generations

**Files:**

- Modify: `src/feeds/repository.rs`
- Modify: `src/feeds/refresh.rs`
- Create: `tests/feed_entry_persistence.rs`

**Interfaces:**

- A persist transaction first verifies database-clock owner/token fencing, then compares existing identities, increments `INGEST_GENERATION` only when at least one new entry exists, allocates monotonic feed sequences, updates changed content, updates Feed/run state, and releases the lease.
- `(feed_id, identity_hash)` is final duplicate arbitration; a hit compares the full identity and returns `IdentityHashCollision` if different.
- Existing rows preserve ID, inserted time, ingest generation, feed sequence, and sort key. Tracking-only sanitized equality avoids a large-field rewrite. `sort_at_us` is derived from checked `published_at_us` or insertion time once and never changes.
- Backend-specific exact unique-conflict/upsert functions are private. Broad MySQL `INSERT IGNORE` is forbidden. Transaction retry reuses parsed content and never repeats network I/O.

- [ ] **Step 1: Add one compiling persist input/output seam**

Define owned `PersistFeed`, `PersistEntry`, and `PersistResult` types. Begin with one new-entry test that compiles and fails on count/row assertions.

- [ ] **Step 2: Implement behavioral slices**

Run RED/GREEN cycles for: first 60 inserts; second identical refresh inserts zero; tracking-only change updates zero; real content change updates one without changing identity fields/state; concurrent persists leave one identity row; all new rows share one generation and monotonic sequences; no-new-entry refresh does not increment the counter; a stale token writes nothing.

- [ ] **Step 3: Verify persistence**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_entry_persistence -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test feed_entry_persistence postgres -- --nocapture
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test feed_entry_persistence mysql -- --nocapture
```

- [ ] **Step 4: Commit**

```bash
git add src/feeds tests/feed_entry_persistence.rs
git commit -m "feat: persist feed entries idempotently"
```

---

### Task 8: Transactional lifecycle outbox records

**Files:**

- Create: `src/db/entities/lifecycle_outbox.rs`
- Modify: `src/db/entities.rs`
- Create: `src/db/migration/rss/outbox.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Modify: `src/feeds/repository.rs`
- Create: `tests/feed_lifecycle_outbox.rs`

**Interfaces:**

- Exact columns: `id VARCHAR(36) PK`; `event_type VARCHAR(64) NOT NULL`; `aggregate_type VARCHAR(32) NOT NULL`; `aggregate_id VARCHAR(64) NOT NULL`; `refresh_id VARCHAR(36) NOT NULL` with no FK so retention cannot block delivery; `event_sequence INTEGER NOT NULL`; `payload_version INTEGER NOT NULL`; `payload_json TEXT NOT NULL`; `idempotency_key VARCHAR(128) NOT NULL`; `status VARCHAR(16) NOT NULL DEFAULT 'PENDING'`; `available_at` non-null operational timestamp; `attempts INTEGER NOT NULL DEFAULT 0`; nullable `lease_owner VARCHAR(64)`/`lease_until`; non-null `created_at`; nullable `completed_at`.
- Outbox statuses are `PENDING/DELIVERING/DELIVERED/DEAD`. Named indexes: `uq_lifecycle_outbox_idem(idempotency_key)`, `uq_lifecycle_outbox_order(refresh_id,event_sequence)`, and `idx_lifecycle_outbox_due(status,available_at,lease_until,id)`.
- Event order uses sequence 10 for `feed.refresh.persisted` and 20 for `feed.refresh.completed`; keys are `refresh:{id}:persisted:v1` and `refresh:{id}:completed:v1`.
- A successful/partial 200 persist writes both events in the same transaction as entries/feed/run. 304 and owned pre-persist errors write only completed. A stale `LEASE_LOST` worker writes no outbox event. This task records reliably but does not implement a dispatcher or claim delivery.

- [ ] **Step 1: Add entity/migration and failing event-matrix tests**

Test successful 200, partial 200, 304, owned error, lease lost, transaction rollback, duplicate retry, payload version/size validation, and deterministic persisted-before-completed ordering.

- [ ] **Step 2: Implement transactional recording**

Payload is capped at 64 KiB, schema-versioned canonical JSON and contains stable IDs/counts/error codes only. It contains no raw Feed body, URL query, validator, stack, or unsanitized HTML.

- [ ] **Step 3: Verify outbox records**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_lifecycle_outbox -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test feed_lifecycle_outbox postgres -- --nocapture
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test feed_lifecycle_outbox mysql -- --nocapture
```

- [ ] **Step 4: Commit**

```bash
git add src/db src/feeds tests/feed_lifecycle_outbox.rs
git commit -m "feat: record feed lifecycle events"
```

---

### Task 9: Deterministic ingestion service and IT Home live smoke

**Files:**

- Create: `src/feeds/service.rs`
- Create: `src/feeds/dto.rs`
- Create: `src/feeds/query.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_ingestion_e2e.rs`
- Create: `tests/live_rss_ithome.rs`
- Modify: `tasks/todo.md`
- Create: `docs/rss-security.md`

**Interfaces:**

- `FeedService<T: FeedTransport>` composes URL policy, injected transport, parser/sanitizer, repository, and refresh scheduler. Callers receive typed subscription/refresh/query DTOs; no caller handles raw HTTP or database entities.
- `subscribe(user_id, SubscribeInput { url }) -> Result<SubscriptionDto, FeedServiceError>` creates/reuses the shared Feed, creates the user-owned Subscription with start/read-through equal to the current feed head, and queues/executes a `SUBSCRIBE` refresh through the same refresh path.
- `EntryRepository::list_for_user(user_id, ListEntriesQuery) -> Result<EntryPage, RepositoryError>` uses subscription joins and a versioned base64url cursor containing `v=1`, `filter_hash`, `snapshot_generation`, `sort_at_us`, and `entry_id`. `get_detail_for_user(user_id, entry_id)` must join a Subscription; opaque entry IDs never authorize access.
- The deterministic E2E path injects a fake `FeedTransport` that returns the synthetic 60-item bounded document: subscribe -> parse -> sanitize -> persist -> user-scoped list/detail DTO -> second refresh idempotency. It does not require a production loopback bypass.
- The IT Home smoke is `#[ignore]`, also checks `RAINDROP_LIVE_RSS_SMOKE=1`, uses a temporary SQLite database and at most two requests to `https://www.ithome.com/rss/`, and never fetches article pages/images.
- Live smoke hard assertions are successful secure parse, 50..=100 entries, unique identities, safe sanitized content, list/detail visibility, and second-refresh database deduplication. Current 60-item size, Brotli ratio, and Last-Modified/304 are diagnostic expectations, not permanent exact values.

- [ ] **Step 1: Write the deterministic failing vertical test**

The test creates two users, subscribes both through production `FeedService` with a fake `FeedTransport`, returns the synthetic 60-item response twice, and asserts:

```text
one Feed row, two Subscription rows, sixty Entry rows
both users can query the same entry content through user-scoped repository DTOs
second refresh has zero duplicates and stable entry IDs/inserted_at
sanitized detail contains no raw XML, script/style/on*/iframe/form/SVG, publisher class/data/style, or active remote image src
```

- [ ] **Step 2: Implement `FeedService` and pass deterministic E2E**

```bash
cargo test --locked --test feed_ingestion_e2e -- --nocapture
```

Expected: PASS without public network access.

- [ ] **Step 3: Add ignored live smoke and run it explicitly**

Create the exact test `ithome_feed_securely_ingests_and_deduplicates` with `#[tokio::test]` and `#[ignore = "requires RAINDROP_LIVE_RSS_SMOKE=1 and public network"]`. Its first executable check must require `std::env::var("RAINDROP_LIVE_RSS_SMOKE").as_deref() == Ok("1")` and panic with `set RAINDROP_LIVE_RSS_SMOKE=1 to run the live RSS smoke` otherwise. The remaining body uses production `FeedService`, temporary SQLite, two refreshes, and only the fixed user-provided URL.

Run:

```bash
RAINDROP_LIVE_RSS_SMOKE=1 cargo test --locked --test live_rss_ithome -- --ignored --nocapture
```

Expected: PASS; first successful representation has 50..=100 unique entries, current observation about 60; immediate second refresh is 304 or a deduplicated 200.

- [ ] **Step 4: Document security boundaries and update evidence checklist**

`docs/rss-security.md` records scheme policy, SSRF address classes, DNS pinning, redirect rules, body/time limits, XML preflight, sanitizer policy, error redaction, and the exact opt-in smoke command. Mark only completed RSS checklist items; do not mark scheduling/API/Reader work that this plan has not delivered.

- [ ] **Step 5: Run the complete data-ingestion gate**

```bash
cargo install cargo-audit --locked --version 0.22.2
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo tree --locked -e features -i reqwest
! cargo tree --locked -e features | rg 'system-proxy|cookies|native-tls|http3'
cargo audit
git diff --check
```

Expected: every command exits 0. Run the IT Home smoke separately and record its date/count/304 behavior without copying Feed content into the repository.

- [ ] **Step 6: Commit**

```bash
git add src/feeds tests/feed_ingestion_e2e.rs tests/live_rss_ithome.rs tasks/todo.md docs/rss-security.md
git commit -m "test: verify rss ingestion end to end"
```

## Plan self-review

- Spec coverage: this plan covers portable Feed/Subscription/Entry/EntryState storage, three-database schema contracts, safe URL fetching, conditional requests, parsing, sanitization, idempotent insertion, fenced refresh persistence, lifecycle outbox records, minimal production subscribe/list/detail domain seams, deterministic fixtures, and the required IT Home live smoke. Subscription HTTP routes, scheduler workers, categories, retention, OPML, and Reader UI intentionally remain in later independently runnable plans.
- DDIA: shared records, sparse user state, stable snapshot generation, feed sequence, unique constraints, fencing token, short transactions, database-as-record-system, and at-least-once outbox semantics are explicit.
- Security: every external boundary has limits and abuse tests; automatic proxy/redirect/decompression are disabled; DNS is pinned; XML and HTML are treated as hostile; logs and errors do not expose URL query, validators, body, or secrets.
- Type consistency: Task 1 produces entities verified across backends by Task 2; Task 3 produces URL/identity/schedule types consumed by Task 4 and later repositories; Task 4 produces bounded documents consumed by Task 5; Task 5 produces validated parsed content consumed by Tasks 7 and 9; Task 6 produces fenced refresh claims; Task 7 persists entries/generations; Task 8 records lifecycle events; Task 9 composes the stable transport/domain/query interfaces.
- Unresolved-marker scan: clean; later subsystems are named as explicit exclusions.
