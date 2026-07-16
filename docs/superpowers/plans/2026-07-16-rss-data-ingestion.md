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
- `feeds`: UUID-string `id`; source/normalized/fetch URLs; URL hash; validators bound to `validator_url`; response hash; monotonic `entry_sequence_head`; attempt/success/changed/next/retry timestamps; failure/error/disabled/orphan fields; `lease_owner`, monotonic `lease_token`, and `lease_until`; created/updated timestamps.
- `subscriptions`: UUID-string `id`, `user_id`, `feed_id`, display override/position, `start_sequence`, `read_through_sequence`, `state_revision`, created/updated timestamps. Unique `(user_id, feed_id)` and unique `(user_id, feed_id, id)`.
- `entries`: UUID-string `id`, `feed_id`, immutable `feed_sequence`, immutable `ingest_generation`, identity kind/full/hash, canonical URL, title/author, sanitized content/summary, published time, immutable `sort_at_us`, inserted/updated times, source/final content hashes, pipeline version, direction, and versioned enclosure JSON text.
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

Use explicitly named constraints/indexes:

```text
uq_feeds_url_hash(normalized_url_hash)
idx_feeds_due(is_disabled,next_fetch_at,lease_until,id)
uq_subscriptions_user_feed(user_id,feed_id)
uq_subscriptions_owner_tuple(user_id,feed_id,id)
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

### Task 2: URL, address, validator, entry identity, and scheduling primitives

**Files:**

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
- `RefreshSchedule::after_result(now, result, failures, retry_after)` implements 5-minute base, full jitter injected by a testable source, and 4-hour maximum.

- [ ] **Step 1: Write failing table-driven primitive tests**

Cover:

```text
HTTPS normalization: host case, IDNA, root dot, default port, empty path, fragment removal, duplicate query preservation
Rejection: credentials, controls, oversized input, relative/network-path URL, unsupported scheme, malformed port
HTTP matrix: default reject; explicit insecure policy accepts HTTP; HTTPS-to-HTTP redirect rejects
Addresses: private, loopback, link-local, CGNAT, metadata, documentation, multicast, unspecified, reserved, IPv4-mapped IPv6, NAT64/6to4/Teredo embedding denied IPv4
Identity: opaque GUID, URL GUID normalization, canonical URL fallback, deterministic fingerprint fallback, fetch-time independence
Validators: same exact URL allowed; changed final URL or origin does not reuse
Schedule: success/304 reset; transient exponential backoff with deterministic jitter; Retry-After bounded to 4 hours
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

Normalization removes fragments/default ports/root dot, preserves query ordering, rejects credentials, and hashes the complete normalized URL. Address ranges are a fixed, reviewed CIDR table backed by `ipnet`; tests name every denied class. Fingerprints use a domain-separated BLAKE3 hasher and normalized stable text.

- [ ] **Step 4: Verify primitives**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_primitives -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/feeds src/lib.rs tests/feed_primitives.rs
git commit -m "feat: define safe feed primitives"
```

---

### Task 3: Pinned, bounded HTTP fetch transport

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/feeds/resolver.rs`
- Create: `src/feeds/fetch.rs`
- Create: `src/feeds/decode.rs`
- Create: `src/feeds/test_server.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_fetch_security.rs`

**Interfaces:**

- Add `reqwest = { version = "0.13.4", default-features = false, features = ["rustls", "stream"] }`.
- Add `async-compression = { version = "0.4.42", default-features = false, features = ["tokio", "gzip", "zlib", "brotli"] }`, `futures-util = "0.3.32"`, and `ipnet = "2.12.0"` if Task 2 did not add it.
- `DnsResolver` returns all A/AAAA `IpAddr` values under a 3-second deadline. Production uses Tokio lookup; tests inject a fake resolver.
- `FeedFetcher::fetch(FetchRequest) -> Result<FetchOutcome, FeedFetchError>` returns `NotModified` or a bounded decoded response with final URL, validator metadata, MIME, raw decoded hash, byte counts, and hop diagnostics.
- Each hop builds a short-lived reqwest client with `redirect(Policy::none())`, `no_proxy()`, 5-second connect timeout, remaining total timeout, and `resolve_to_addrs(host, approved_socket_addrs)`.
- At most 16 DNS results and five redirects. Any denied address rejects the whole set. Every redirect repeats URL/DNS/address validation and never forwards validators across a changed validator URL.
- Automatic decompression is disabled. Compressed bytes are capped at 2 MiB; decoded bytes at 10 MiB; ratio at 100:1; unknown or layered encodings reject.

- [ ] **Step 1: Write failing transport abuse tests**

Use fake resolver/recording transport tests for rebinding and a local scripted server only through `#[cfg(test)] FeedUrlPolicy::test_loopback()`.

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
```

- [ ] **Step 2: Run tests and verify transport is absent**

```bash
cargo test --test feed_fetch_security -- --nocapture
```

Expected: FAIL before dependencies and transport exist.

- [ ] **Step 3: Add minimal dependencies and implement the transport**

Do not enable reqwest `default`, `system-proxy`, `cookies`, compression, native TLS, HTTP/2, or HTTP/3 features. Implement manual handling for 301/302/303/307/308 and explicit status classification for 200, 304, 204, 401/403, 404/410, 408/425/429, 5xx, and other 4xx. `Location` may be relative and is resolved against the current URL before full revalidation.

Decoded bodies remain bytes; do not create an unbounded `String`. Invalid header values are not reflected in errors. Diagnostics contain feed ID/host/status/counts only, never full URL query, validator, or body.

- [ ] **Step 4: Audit dependency features and run transport tests**

```bash
cargo tree -e features | rg "reqwest|proxy|cookie|native-tls|http3|async-compression"
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test feed_fetch_security -- --nocapture
```

Expected: PASS; tree shows rustls/stream and only Brotli/gzip/zlib codecs, without system proxy, cookie jar, native-tls, or HTTP/3.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/feeds tests/feed_fetch_security.rs
git commit -m "feat: fetch feeds through pinned transport"
```

---

### Task 4: Feed parsing, XML preflight, and HTML sanitization

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
- `preflight_xml(&[u8])` rejects DTD/ENTITY, nesting deeper than 128, and pathological event/attribute growth before feed-rs parses.
- `parse_feed(FetchedDocument) -> Result<ParsedFeed, FeedParseError>` supports RSS/Atom/JSON Feed, caps 5,000 entries, and retains `published_at=None` when the source has no date.
- `sanitize_entry_html(base_url, input) -> SanitizedContent` removes scripts/styles/forms/frames/SVG/MathML, event handlers, style/class/id/data attributes, unsafe schemes, `srcset`, and tracking attributes.
- Anchors retain only safe HTTP(S) hrefs. Images become inert placeholders that retain safe original URL metadata and alt/width/height but no active remote `src`.
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
```

- [ ] **Step 2: Run tests and verify parser/sanitizer are absent**

```bash
cargo test --test feed_parse_sanitize -- --nocapture
```

Expected: FAIL before parser/sanitizer modules exist.

- [ ] **Step 3: Implement preflight, parsing, mapping, and sanitizer policy**

MIME handling accepts XML/JSON Feed MIME directly. `text/plain`, `text/html`, and `application/octet-stream` parse only when BOM/whitespace-stripped body sniff begins with XML/RSS/Atom/RDF or a JSON object. Binary/image/audio/video/PDF bodies reject.

Feed mapping resolves relative links against `xml:base` or final response URL, then restricts them to credential-free HTTP(S) and removes fragments. Duplicate identities within one document choose the newer source timestamp, then stable document order.

- [ ] **Step 4: Verify corpus and dependency graph**

```bash
cargo tree -e features | rg "feed-rs|quick-xml|ammonia"
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test feed_parse_sanitize -- --nocapture
```

Expected: PASS; feed-rs `sanitize` feature is absent.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/content src/feeds src/lib.rs tests/fixtures tests/feed_parse_sanitize.rs
git commit -m "feat: parse and sanitize feed content"
```

---

### Task 5: Fenced refresh runs, idempotent persistence, and lifecycle outbox

**Files:**

- Create: `src/db/entities/feed_refresh_run.rs`
- Create: `src/db/entities/lifecycle_outbox.rs`
- Modify: `src/db/entities.rs`
- Create: `src/db/migration/rss/refresh_runs.rs`
- Create: `src/db/migration/rss/outbox.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Create: `src/feeds/repository.rs`
- Create: `src/feeds/refresh.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_refresh_repository.rs`

**Interfaces:**

- `FeedRepository::claim_due` uses a conditional update that increments `lease_token`; affected rows must be one.
- `RefreshClaim` carries feed ID, owner, token, refresh ID, trigger, and lease deadline.
- Every success/failure/304 commit verifies feed ID + owner + token + unexpired lease. A stale worker returns `LeaseLost` and writes no Feed/Entry result.
- New entries in one refresh share one newly incremented `INGEST_GENERATION`; each receives the next immutable feed sequence. No new entries means no generation increment.
- `(feed_id, identity_hash)` is the final duplicate boundary; a hash hit must compare full identity. Same content avoids rewriting large fields; changed sanitized content updates the existing row but preserves ID, inserted time, generation, sequence, and sort key.
- `feed_refresh_runs` records queued/running/success/not-modified/partial/error/lease-lost status with stable idempotency key and counts.
- `lifecycle_outbox` stores versioned `feed.refresh.persisted` then `feed.refresh.completed` events in the same transaction as persisted data. Delivery is at least once; no external work runs before commit.

- [ ] **Step 1: Write failing repository/concurrency contracts**

Required tests:

```rust
two_users_share_one_feed_and_one_entry_set()
first_refresh_inserts_sixty_and_second_refresh_inserts_zero()
tracking_only_refresh_does_not_update_content()
real_content_change_updates_existing_entry_without_losing_state()
concurrent_refreshes_leave_one_identity_row()
stale_lease_token_cannot_commit_after_new_worker_claims()
new_entries_share_one_generation_and_monotonic_sequences()
persisted_and_completed_outbox_events_are_transactional_and_idempotent()
```

- [ ] **Step 2: Run tests and verify repository is absent**

```bash
cargo test --test feed_refresh_repository -- --nocapture
```

Expected: FAIL before refresh repository/migrations exist.

- [ ] **Step 3: Implement short transactions and precise conflicts**

Do not use broad MySQL `INSERT IGNORE`. Wrap backend-specific exact unique-conflict/upsert details behind private repository functions and keep the public repository contract identical. Network/parse output enters as owned validated `ParsedFeed`; transaction retries reuse that parsed value and never repeat the network request.

`sort_at_us` is fixed on first insert from a reasonable source date or insertion time. Future source dates are clamped by an explicit policy and later publisher changes never reorder an existing row.

- [ ] **Step 4: Verify repository behavior**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test feed_refresh_repository -- --nocapture
cargo test --test rss_migrations -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/db src/feeds tests/feed_refresh_repository.rs tests/rss_migrations.rs
git commit -m "feat: persist fenced feed refreshes"
```

---

### Task 6: Deterministic ingestion service and IT Home live smoke

**Files:**

- Create: `src/feeds/service.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_ingestion_e2e.rs`
- Create: `tests/live_rss_ithome.rs`
- Modify: `tasks/todo.md`
- Create: `docs/rss-security.md`

**Interfaces:**

- `FeedService` composes URL policy, resolver/fetcher, parser/sanitizer, repository, and refresh scheduler. Callers receive typed `RefreshResult`; no caller handles raw HTTP or database entities.
- The deterministic E2E path is synthetic 60-item RSS: fetch -> decode -> parse -> sanitize -> persist -> query list/detail DTO -> second refresh idempotency.
- The IT Home smoke is `#[ignore]`, also checks `RAINDROP_LIVE_RSS_SMOKE=1`, uses a temporary SQLite database and at most two requests to `https://www.ithome.com/rss/`, and never fetches article pages/images.
- Live smoke hard assertions are successful secure parse, 50..=100 entries, unique identities, safe sanitized content, list/detail visibility, and second-refresh database deduplication. Current 60-item size, Brotli ratio, and Last-Modified/304 are diagnostic expectations, not permanent exact values.

- [ ] **Step 1: Write the deterministic failing vertical test**

The test creates two users, subscribes both to one normalized URL, runs the local scripted 60-item response twice, and asserts:

```text
one Feed row, two Subscription rows, sixty Entry rows
both users can query the same entry content through user-scoped repository DTOs
second refresh has zero duplicates and stable entry IDs/inserted_at
sanitized detail contains no raw XML, script/style/on*/iframe/form/SVG, publisher class/data/style, or active remote image src
```

- [ ] **Step 2: Implement `FeedService` and pass deterministic E2E**

```bash
cargo test --test feed_ingestion_e2e -- --nocapture
```

Expected: PASS without public network access.

- [ ] **Step 3: Add ignored live smoke and run it explicitly**

Create the exact test `ithome_feed_securely_ingests_and_deduplicates` with `#[tokio::test]` and `#[ignore = "requires RAINDROP_LIVE_RSS_SMOKE=1 and public network"]`. Its first executable check must require `std::env::var("RAINDROP_LIVE_RSS_SMOKE").as_deref() == Ok("1")` and panic with `set RAINDROP_LIVE_RSS_SMOKE=1 to run the live RSS smoke` otherwise. The remaining body uses production `FeedService`, temporary SQLite, two refreshes, and only the fixed user-provided URL.

Run:

```bash
RAINDROP_LIVE_RSS_SMOKE=1 cargo test --test live_rss_ithome -- --ignored --nocapture
```

Expected: PASS; first successful representation has 50..=100 unique entries, current observation about 60; immediate second refresh is 304 or a deduplicated 200.

- [ ] **Step 4: Document security boundaries and update evidence checklist**

`docs/rss-security.md` records scheme policy, SSRF address classes, DNS pinning, redirect rules, body/time limits, XML preflight, sanitizer policy, error redaction, and the exact opt-in smoke command. Mark only completed RSS checklist items; do not mark scheduling/API/Reader work that this plan has not delivered.

- [ ] **Step 5: Run the complete data-ingestion gate**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo tree -e features | rg "reqwest|feed-rs|quick-xml|ammonia|async-compression|proxy|cookie|native-tls|http3"
git diff --check
```

Expected: every command exits 0. Run the IT Home smoke separately and record its date/count/304 behavior without copying Feed content into the repository.

- [ ] **Step 6: Commit**

```bash
git add src/feeds tests/feed_ingestion_e2e.rs tests/live_rss_ithome.rs tasks/todo.md docs/rss-security.md
git commit -m "test: verify rss ingestion end to end"
```

## Plan self-review

- Spec coverage: this plan covers portable Feed/Subscription/Entry/EntryState storage, safe URL fetching, conditional requests, parsing, sanitization, idempotent insertion, fenced refresh persistence, lifecycle outbox, deterministic fixtures, and the required IT Home live smoke. Subscription HTTP APIs, scheduler workers, categories, retention, OPML, and Reader UI intentionally remain in later independently runnable plans.
- DDIA: shared records, sparse user state, stable snapshot generation, feed sequence, unique constraints, fencing token, short transactions, database-as-record-system, and at-least-once outbox semantics are explicit.
- Security: every external boundary has limits and abuse tests; automatic proxy/redirect/decompression are disabled; DNS is pinned; XML and HTML are treated as hostile; logs and errors do not expose URL query, validators, body, or secrets.
- Type consistency: Task 1 produces entities consumed by Task 5; Task 2 produces URL/identity/schedule types consumed by Tasks 3 and 5; Task 3 produces bounded documents consumed by Task 4; Task 4 produces validated parsed content consumed by Task 5; Task 6 composes those stable interfaces.
- Unresolved-marker scan: clean; later subsystems are named as explicit exclusions.
