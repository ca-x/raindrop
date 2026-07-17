# Raindrop RSS Data Ingestion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the trustworthy RSS record path from a subscription URL to shared Feed/Entry rows, including portable schema, secure network fetching, parsing, HTML sanitization, fenced refresh persistence, deterministic fixtures, and an opt-in IT Home live smoke.

**Architecture:** The relational database remains the record system. Feeds and entries are shared globally; subscriptions and sparse entry state are user-owned. Network, decompression, parsing, sanitization, and synchronous content processing happen outside database transactions. Short transactions use unique constraints, monotonic feed sequences, a global ingest generation, and lease fencing tokens for idempotent persistence.

**Tech Stack:** Rust 1.94, Axum 0.8, Tokio 1.52, SeaORM/sea-orm-migration declared as 1.1.19 and locked to 1.1.20, reqwest 0.13.4 with `rustls-no-provider` and stream plus direct rustls ring, feedparser-rs =0.5.5 without its default `http` feature, quick-xml =0.41.0, ammonia =4.1.3, encoding_rs =0.8.35, html5ever =0.39.0, mime =0.3.17, async-compression 0.4.42, ipnet 2.12.0, BLAKE3, SQLite/PostgreSQL/MySQL.

## Global Constraints

- Rust edition 2024 and MSRV 1.94; committed `Cargo.lock` is authoritative and all Cargo verification uses `--locked` after dependency resolution.
- Feed and Entry are shared records. Subscription and EntryState are always scoped by an authenticated `user_id`.
- Use `ingest_generation + feed_sequence`, not timestamps, for stable list and mark-read snapshots.
- Every refresh lease has a monotonically increasing `lease_token`; a worker that loses owner/token fencing cannot commit.
- Feed identity and entry identity use BLAKE3 hash indexes plus full normalized-value comparison; hash collision is an explicit error, never a silent merge.
- Do not execute network, XML/JSON parsing, HTML sanitization, plugins, AI, MCP, or notifications inside a database transaction.
- Default subscription policy accepts HTTPS only. HTTP requires an instance-level `allow_insecure_http` policy; HTTPS redirects may never downgrade to HTTP.
- Disable automatic redirects, ambient/system proxy use, and automatic response decompression. Revalidate and pin DNS addresses on every redirect hop, with at most five redirects.
- `feedparser-rs` is parser-only: keep its default `http` feature disabled and never call parser-owned URL fetching. Call `parse_with_limits` only after Raindrop MIME/encoding/XML preflight, reject unknown format and every `bozo` result, and immediately map crate types into owned domain types.
- Default limits are DNS 3 s, connect/TLS 5 s, first byte 10 s, body idle 10 s, one hop 20 s, whole refresh 30 s, compressed body 2 MiB, decoded body 10 MiB, compression ratio 100:1, 5,000 entries, and XML depth 128.
- Server-side HTML sanitization is mandatory. React never receives raw Feed XML or unsanitized publisher HTML; remote images remain inert by default.
- Persisted sanitized content uses the canonical `rdsc:v1:` envelope: the prefix followed by compact JSON with exact top-level keys `html` and `inertImages`; each image record uses `imageIndex`, `sourceUrl`, `alt`, `width`, and `height` in that order. Semantic `source_content_hash` and `content_hash` remain hashes of sanitized HTML only, never image source URLs or the storage envelope.
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
- Modify: `src/db/migration.rs`
- Modify: `src/db/migration/rss/feeds.rs`
- Modify: `src/db/migration/rss/subscriptions.rs`
- Modify: `src/db/migration/rss/entries.rs`
- Modify: `src/db/migration/rss/entry_states.rs`
- Modify: `tests/rss_migrations.rs`
- Create: `tests/support/mod.rs`
- Create: `tests/support/database.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/configuration.md`

**Interfaces:**

- One `rss_schema_contract(database_url)` test body runs against mandatory temporary SQLite and opt-in `RAINDROP_TEST_POSTGRES_URL` / `RAINDROP_TEST_MYSQL_URL` connections.
- CI provides PostgreSQL and MySQL services and runs the same contract for both; local developers may omit those environment variables.
- Connection initialization configures every future pool connection at handshake: `map_sqlx_postgres_opts(|opts| opts.options([("timezone", "UTC")]))` and `map_sqlx_mysql_opts(|opts| opts.timezone(Some("+00:00".to_owned())))`. Tests acquire more than one pool connection and verify UTC/microsecond round trips for operational `OffsetDateTime` fields.
- SQLite connection initialization also uses `map_sqlx_sqlite_opts` for `foreign_keys`, a five-second `busy_timeout`, `synchronous=NORMAL`, and file-only WAL. With its pool limit of one, the contract closes the first acquisition and verifies a replacement connection inherits the same options.
- Backend-aware RSS migration columns preserve operational timestamp precision and range: MySQL uses explicit `DATETIME(6)`, PostgreSQL uses `TIMESTAMPTZ`, and SQLite keeps the existing portable timestamp type. Roundtrip fixtures are exactly microsecond-aligned; a later repository write helper truncates arbitrary operational timestamps to microseconds before persistence.
- Untrusted source dates remain `published_at_us: Option<i64>` and never enter MySQL `TIMESTAMP`. Parsing rejects values outside signed Unix-microsecond representation; display conversion is outside SQL.
- MySQL migration reentry is proven by a partial-state contract: precreate a target table/index state, rerun the corresponding migration path, and assert all expected named indexes/seed rows exist without broad `INSERT IGNORE`.
- `INGEST_GENERATION` seed uses an exact primary-key conflict path and validates an existing row's value/type; unrelated database errors remain visible.

- [ ] **Step 1: Extract a backend-parameterized schema contract**

Move fixed user/feed/subscription/entry/state setup into `tests/support/database.rs`. The public test seam is `db::migrate` plus SeaORM entity/constraint behavior. SQLite always runs; PostgreSQL/MySQL tests are marked skipped only when their environment URL is absent and print no credentials.

- [ ] **Step 2: Add failing UTC/range/reentry cases**

Add exact cases for operational timestamp UTC microsecond roundtrip, publisher dates before 1970 and after 2038 through `published_at_us`, different full identities with different hashes coexisting under MySQL case-insensitive collation while the same hash remains unique, and partial index/seed migration recovery. Full-value comparison after hash lookup and typed hash-collision rejection remain Task 3/7 ingestion behavior.

- [ ] **Step 3: Configure backend sessions and portable migration reentry**

Configure SQLite/PostgreSQL/MySQL connect options before pool creation so every newly opened connection receives its required session settings; one-time post-connect `PRAGMA`/`SET` statements are insufficient. Never include a database URL in logs/errors. Update migration helpers so standalone named indexes call `has_index` before creation where required and the counter seed handles only the exact primary-key conflict.

- [ ] **Step 4: Add CI services and run all three contracts**

Run locally for SQLite, then in CI with PostgreSQL/MySQL service URLs:

```bash
cargo test --locked --test rss_migrations -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test rss_migrations postgres -- --nocapture --test-threads=1
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test rss_migrations mysql -- --nocapture --test-threads=1
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Expected: the same contract passes on all three backends without printing connection secrets.

- [ ] **Step 5: Commit**

```bash
git add src/db/connect.rs src/db/migration.rs src/db/migration/rss tests/rss_migrations.rs tests/support .github/workflows/ci.yml docs/configuration.md docs/superpowers/plans/2026-07-16-rss-data-ingestion.md
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

- `FeedUrlPolicy::new(allow_insecure_http: bool)` validates and normalizes absolute URLs whose raw and normalized UTF-8 forms are each at most 4,096 bytes. It rejects raw C0/space/DEL and Unicode control characters before `url::Url` can trim them.
- `NormalizedFeedUrl` owns the complete normalized URL, lower-case BLAKE3 hash, canonical host, scheme, and effective port. The complete URL field is private; public accessors expose only non-sensitive components, the transport gets a `pub(crate)` URL accessor, and the type implements neither `Display` nor `Serialize`. Custom `Debug` contains no path, query, fragment, or userinfo.
- `AddressPolicy::public_only()` returns `Allowed` only for reviewed globally routable unicast addresses. `AddressPolicy::with_nat64_prefixes(...)` accepts only trusted, canonical, non-overlapping RFC 6052 `/32`, `/40`, `/48`, `/56`, `/64`, or `/96` prefixes contained in allowed native global-unicast space; invalid, special-range, host-bit-bearing, or ambiguous prefixes are typed constructor errors. IPv4-mapped/compatible, well-known NAT64, configured NAT64, 6to4, and both Teredo IPv4 components are unwrapped before the embedded addresses are classified.
- `EntryIdentity::from_parts(guid, canonical_url, StableEntryFields)` uses `GUID -> URL -> FINGERPRINT`; fetch time and document position are never identity inputs. `StableEntryFields` has a fixed v1 byte grammar, and both the fallback fingerprint and final index hash use versioned BLAKE3 derive-key contexts with the identity kind included.
- `ValidatorSet` is reusable only when its exact complete normalized `validator_url`, including query, matches the request URL. `OpaqueValidator` validates `http::HeaderValue`, accepts `1..=8,192` bytes, preserves arbitrary valid header bytes, stores canonical `v1:` URL-safe unpadded base64 in the existing database `TEXT` columns, marks reconstructed values sensitive, and always redacts `Debug`.
- `RefreshSchedule<J: JitterSource>::after_result(now, previous_failures, result)` validates the signed persisted previous count and owns the increment/reset rule. `RefreshResult::Success` and `NotModified` carry no retry state; `TransientFailure` carries only an optional `RetryAfter`. It returns `ScheduleOutcome { next_at, consecutive_failures, retry_after_at }`.
- `RetryAfter::parse(raw, received_at)` parses delta-seconds or any HTTP-date form accepted for compatibility by `httpdate` (IMF-fixdate, obsolete RFC 850, or asctime) into an absolute UTC instant. Delta-seconds are anchored to response receipt time; past dates become zero delay at scheduling time. The final delay is `min(max(full_jitter, retry_after_delay), 4 hours)`.

**Task 3 internal design freeze (2026-07-16):**

- Threat boundary: subscription URLs, DNS answers, redirects, publisher GUIDs/links, validators, dates, and stable entry text are attacker-controlled. The protected assets are loopback/private/metadata services, URL query tokens, validator bytes, stable entry identity, scheduler availability, and bounded memory/CPU.
- URL normalization validates the raw bytes first. Within the raw authority (`://` through the first `/`, `?`, `#`, or `\\`), any literal `@` rejects userinfo including empty usernames, and an explicit empty port suffix rejects before `url` can erase it; `@`/`%40` in path or query are not credentials. It preserves duplicate query keys and their ordering without rebuilding through `query_pairs`, removes fragments/default ports/a single domain root dot, and rechecks the normalized byte length. After locked `url 2.5.8`/`idna 1.1.0` ASCII serialization, every domain label must be strict LDH, `1..=63` bytes, begin/end alphanumeric, contain no empty internal label, and the root-dot-free host is at most 253 bytes. IP literals bypass DNS-label checks. Non-standard IPv4 text such as `127.1`, `0x7f000001`, and `2130706433` must normalize before address classification and still be denied.
- The fixed address policy is based on the IANA IPv4/IPv6 Special-Purpose Address Registry snapshot retrieved 2026-07-16. Native IPv4 denies `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10`, `127.0.0.0/8`, `169.254.0.0/16`, `172.16.0.0/12`, `192.0.0.0/24`, `192.0.2.0/24`, `192.88.99.0/24`, `192.168.0.0/16`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`, `224.0.0.0/4`, and `240.0.0.0/4`; only the complement is allowed. Native IPv6 allows only `2000::/3`, then denies `2001::/23`, `2001:db8::/32`, and `3fff::/20`; transition forms are handled before this native rule.
- IPv6 classification order is exact: IPv4-mapped/compatible; well-known `64:ff9b::/96`; validated configured RFC 6052 prefixes; 6to4 `2002::/16`; Teredo `2001::/32` with both server and inverted client IPv4 classified; unconditional deny for local-use `64:ff9b:1::/48`; fixed special/native IPv6 rules. A configured prefix must be canonical (`network == supplied address`), must not overlap another configured prefix or any mapped/WKP/local-use/6to4/Teredo/fixed-deny range, and must be contained in the allowed native IPv6 set.
- RFC 6052 extraction is fixed by prefix length: `/32` uses bits `32..64`; `/40` uses `40..64 + 72..80`; `/48` uses `48..64 + 72..88`; `/56` uses `56..64 + 72..96`; `/64` uses `72..104`; `/96` uses `96..128`. For `/32` through `/64`, candidate-address bits `64..72` are the `u` octet and must be zero; a matching prefix with non-zero `u` is denied rather than reclassified as native IPv6. A supplied `/96` prefix itself must also have bits `64..72 == 0` or construction fails. Suffix bits after the embedded IPv4 do not affect classification.
- Custom DNS64/NAT64 cannot be inferred from an arbitrary `IpAddr`. Task 4 uses a separate `Nat64PrefixDiscovery` seam, not `lookup_host`, to issue explicit A and AAAA queries for the absolute FQDN `ipv4only.arpa.` and return answer records, DNS response class, and validity deadline. Default mode is automatic: only `NOERROR` A answers containing both `192.0.0.170` and `192.0.0.171` plus an explicit `NOERROR/NODATA` AAAA response establish cached `NotPresent`; valid synthesized AAAA establish `Present(prefixes)`. The standard `64:ff9b::/96` WKP establishes Present but is omitted from configured prefixes because it is intrinsic to `AddressPolicy`. NXDOMAIN, SERVFAIL, REFUSED, timeout, filtered/missing response, a successful zero-answer lookup, zero/missing TTL, malformed/ambiguous WKA mapping, or custom-prefix validation failure are typed fail-closed states. An operator may instead supply validated static prefixes or explicitly opt out for a known non-DNS64 network.
- Automatic discovery requires the explicit A lookup to contain both `192.0.0.170` and `192.0.0.171` for both `Present` and `NotPresent`. A `Present` snapshot uses `min(A Lookup::valid_until, AAAA Lookup::valid_until)`; a `NotPresent` snapshot uses `min(A Lookup::valid_until, checked now + nonzero AAAA negative TTL)`. Every async snapshot-lock read is followed by a new monotonic clock sample; a snapshot is expired exactly when `now >= valid_until`, and total work rejects when `now >= total_deadline`. There is no background refresh or subjective early-refresh window: the first fetch at/after expiry refreshes through a single-flight path before any user-host DNS, and a failed refresh blocks the fetch. Prefix/policy changes atomically replace the complete snapshot and increment generation; a same-policy TTL renewal may keep generation but publishes a new deadline. Attacker-controlled hostnames never provide prefixes.
- Every redirect hop captures the current snapshot generation and exact validity deadline before user-host DNS. After DNS returns and again immediately before the pinned executor starts, the transport atomically rechecks that both values still identify the current unexpired snapshot. A mismatch at either gate discards all DNS results and re-resolves the hostname under the replacement snapshot. The two gates share one replay counter: at most two replays after the initial attempt are allowed per hop, then `Nat64Unstable` is returned. Replays share the original per-hop DNS deadline, the current hop's 20-second deadline, and the 30-second total deadline. No address from a stale snapshot can reach connect.
- Opaque GUIDs are Unicode-whitespace trimmed and otherwise preserved, with raw and normalized identity capped at 64 KiB. A raw value beginning with HTTP(S) syntax that contains userinfo is rejected rather than downgraded to opaque. Absolute credential-free HTTP(S) GUIDs and canonical URLs use a dedicated identity URL normalizer that always accepts HTTP/HTTPS independently of feed-fetch policy, never fetches them, preserves query ordering, removes fragments, and caps normalized URLs at 4,096 bytes. Empty normalized GUIDs are absent.
- Stable text normalization trims leading/trailing `char::is_whitespace`, collapses each internal Unicode-whitespace run to one ASCII space, preserves case and all other UTF-8 scalars, performs no NFC/NFKC normalization, maps empty results to `None`, and caps title/author at 64 KiB. `StableEntryFields` encodes normalized title, author, source `published_at_us`, normalized first enclosure URL, and a 32-byte sanitized-content hash. The content hash field is present only when all first four fields are absent.
- The fingerprint v1 bytes are `RDFP 00 01`, followed in tag order `01=title`, `02=author`, `03=published_at_us`, `04=first_enclosure_url`, `05=content_hash`. Every field is `tag:u8 || present:u8 || len:u32be || value`; absent is `present=0,len=0`, strings are normalized UTF-8, the date is 8-byte two's-complement big-endian, and the content hash is 32 raw bytes. `None` and normalized empty are identical. Fingerprint inputs use derive-key context `raindrop.entry-fingerprint.v1`; the full `FINGERPRINT` identity is the lower-case 64-character digest hex.
- The final index bytes are `RDIX 00 01 || kind:u8 || identity_len:u32be || identity_utf8`, where `01=GUID`, `02=URL`, `03=FINGERPRINT` and database strings are exactly `GUID/URL/FINGERPRINT`. The index hash uses derive-key context `raindrop.entry-identity-index.v1`. A persistence hit compares both `(identity_kind, identity)`, not only the text value.
- Golden vectors are normative: GUID `tag:example.com,2026:42` index bytes `52444958000101000000177461673a6578616d706c652e636f6d2c323032363a3432` hash `e697b4d9b1ce018d8e0ed595b79680c41fdde20685bac778a96724b6380cc13f`; URL `https://example.com/post?a=1&a=2` index bytes `524449580001020000002068747470733a2f2f6578616d706c652e636f6d2f706f73743f613d3126613d32` hash `2d2ef85d39b2644f36462c9e4dd119525683d248e66273bcaea04000f8ad9857`.
- Ordinary fingerprint inputs title `Hello world`, author `Alice`, published `1700000000123456`, no enclosure/content encode as `52444650000101010000000b48656c6c6f20776f726c64020100000005416c69636503010000000800060a2418202240040000000000050000000000`; fingerprint `a0b3e922878ae50dae0e706b4a43aea8438d740ff478a047eb05e869296160ed`; final index hash `2ecb60b16bd41841bd5403adeb3a146ce9304a1fb904a26ac4549849b762788e`. Content-only `[0x11;32]` encodes as `5244465000010100000000000200000000000300000000000400000000000501000000201111111111111111111111111111111111111111111111111111111111111111`; fingerprint `a483e2c16d043f36aa56e3a6c203a76e5e340a4f0713a2632d8cb9f7b1cfc0e3`; final index hash `393eaf5d121df51cc2f4817011fd3e0d593da32a6ac542663c354ce08985d65d`.
- Fallback identity is explicitly best-effort: a correction/addition to title, author, publication time, or enclosure changes the fingerprint; a content-only edit changes its fallback identity. Task 7's in-place content-update guarantee therefore applies only to GUID/URL identities or unchanged fingerprint inputs, and tests preserve this distinction instead of hiding it.
- Validator storage is exactly `v1:` plus `base64::engine::general_purpose::URL_SAFE_NO_PAD` of the original header bytes. Decode rejects unknown versions, padding, whitespace, non-URL-safe alphabet, non-canonical encodings whose re-encoding differs, decoded length outside `1..=8,192`, and bytes rejected by `HeaderValue::from_bytes`; all are typed errors. `OpaqueValidator` marks the first response `HeaderValue` sensitive at construction, and every reconstructed/accessed/cloned `HeaderValue` remains sensitive. Task 7's three-backend persistence contract writes, reloads, decodes, and byte-compares a non-UTF-8 validator.
- `JitterSource` is exactly `fn sample_inclusive_us(&mut self, upper_bound_us: u64) -> u64`; an output above the bound is a typed error. `previous_failures` is a persisted `i64` validated as non-negative for every result. Success and 304 reset it to zero and schedule exactly `now + 5 minutes`. Transient failure saturating-adds one to `i64::MAX`; counts `1..=6` use `5 minutes * 2^(n-1)`, and `n >= 7` uses the four-hour upper bound without shifting. Full jitter is inclusive `[0, upper_bound]`.
- `RetryAfter::parse(&HeaderValue, received_at)` trims HTTP optional whitespace, accepts ASCII delta digits or the three `httpdate` date forms, and stores a UTC absolute instant. Non-ASCII/invalid syntax and decimal overflow past `u64` are typed errors. Because `time 0.3.53` has no `OffsetDateTime::MAX`, a valid delta that cannot be added exactly saturates to `PrimitiveDateTime::MAX.assume_utc()`; past-date tests use representable HTTP dates from 1970 onward and contribute zero delay. Scheduling uses `min(max(jitter_delay, retry_at-now), 4 hours)`, checked-adds the chosen delay to UTC `now`, and returns typed `TimeOverflow` rather than wrapping.
- Only feed-domain and transport/log DTOs promise redacted formatting. SeaORM-generated database models retain macro-provided `Debug` and must never be passed to logs or error payloads; the plan does not claim that their generated formatting is mechanically secret-safe.

- [ ] **Step 1: Write failing table-driven primitive tests**

Cover:

```text
HTTPS normalization: host case, IDNA, root dot, default port, empty path, fragment removal, duplicate query preservation
Rejection: empty userinfo, credentials, explicit empty/malformed port, raw controls/spaces, raw/normalized oversize, invalid DNS labels, relative/network-path URL, unsupported scheme; path/query `@` is preserved and not misclassified
HTTP matrix: default reject; explicit insecure policy accepts HTTP; HTTPS-to-HTTP redirect rejects
Addresses: every exact fixed CIDR boundary, non-standard IPv4 URL text, public positive cases, classification priority, IPv4-mapped/compatible, well-known/configured NAT64, 6to4, and Teredo with both public-allow/private-deny embedded cases
RFC6052: standard vectors for all six prefix lengths, non-canonical/ULA/special/overlapping prefixes, candidate non-zero u octet, `/96` prefix non-zero u octet, suffix variation, WKP/local-use/6to4/Teredo priority
Identity: opaque GUID, URL GUID normalization, credential-like URL rejection, canonical URL fallback, exact v1 encoded bytes, normative golden hashes, concatenation ambiguity, all field normalization boundaries, fetch-time/document-order independence, accepted fallback-change degradation
Validators: same exact URL allowed; changed query/final URL/origin rejected; non-UTF-8 HeaderValue bytes round-trip through canonical `v1:` URL-safe unpadded base64; unknown/corrupt/non-canonical storage; 1/8192/8193-byte boundaries; sensitive HeaderValue and Debug redaction
Schedule: negative/zero/max previous counts; success/304 exact reset; first/sixth/seventh/max transient bounds; inclusive deterministic jitter; invalid jitter; Retry-After delta/three HTTP-date forms/past/skew/u64 overflow/valid-add overflow; checked next-time overflow; 4-hour cap
Redaction: query token, URL GUID, validator bytes, and raw URL never appear in Debug or typed error chains
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

Add `ipnet = { version = "2.12.0", default-features = false }`, `httpdate = "1.0.3"`, and direct `http = { version = "1.4.2", default-features = false }`; reuse the existing direct `base64` dependency. Do not add direct `idna`. Normalization, address classification, validator storage encoding, identity encoding, retry parsing, and scheduling stay in separate focused files with explicit error enums. The lockfile diff should add only the root direct-dependency edges and `ipnet`; existing locked `url 2.5.8`, `idna 1.1.0`, `http 1.4.2`, `httpdate 1.0.3`, and `blake3 1.8.5` remain authoritative.

- [ ] **Step 4: Verify primitives**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_primitives -- --nocapture
cargo +1.94.0 test --locked --test feed_primitives -- --nocapture
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
- Create: `src/feeds/deadline.rs`
- Create: `src/feeds/test_support.rs`
- Create: `tests/feed_tls_provider_conflict.rs`
- Create: `tests/version_fast_path.rs`
- Modify: `src/feeds/mod.rs`

**Interfaces:**

- Add `reqwest = { version = "0.13.4", default-features = false, features = ["rustls-no-provider", "stream"] }` and `rustls = { version = "0.23.42", default-features = false, features = ["ring"] }`. Install the exact ring process provider after the `--version` fast path and before tracing/config/database work, repeat the idempotent check before production reqwest client build, fail hard on a conflicting provider, and keep AWS-LC unreachable.
- Add `async-trait = "0.1.89"`, `async-compression = { version = "0.4.42", default-features = false, features = ["tokio", "gzip", "zlib", "brotli"] }`, `futures-util = "0.3.32"`, and `hickory-resolver = { version = "=0.26.1", default-features = false, features = ["system-config", "tokio"] }`. Hickory 0.26.1 has MSRV 1.88 and supplies explicit record-type lookup, DNS message/rcode inspection, and `Lookup::valid_until`; no DNS-over-TLS/HTTPS/QUIC/DNSSEC feature is enabled. Add Tokio production features `sync` and `time`; add Tokio `test-util` for unit tests without changing production behavior.
- `#[async_trait::async_trait] trait DnsResolver: Send + Sync` returns all explicit A/AAAA `IpAddr` values for attacker-controlled hosts under a 3-second deadline. Production uses the system-configured Hickory resolver; tests inject a fake resolver. Only `NoRecordsFound` plus `NoError` represents an absent family; a successful zero-answer `Lookup`, NXDOMAIN, and every other rcode fail closed.
- A separate `#[async_trait::async_trait] trait Nat64PrefixDiscovery: Send + Sync` returns typed `Present`, `NotPresent`, or error plus a monotonic validity deadline from explicit `ipv4only.arpa.` A/AAAA queries. Its dedicated system-configured Hickory resolver uses `ResolveHosts::Never`, `cache_size=0`, `attempts=1`, leaves all positive/negative TTL min/max options unset, and never shares the user-host resolver cache. A and AAAA execute concurrently inside one three-second timeout and any fetch-triggered refresh is also bounded by the remaining 30-second total deadline. Both outcomes require an A `NOERROR` answer containing both WKAs. `Present` uses the minimum A/AAAA `Lookup::valid_until()`; the standard `64:ff9b::/96` WKP is valid Present but is omitted from configured prefixes because the address policy unwraps it intrinsically. `NotPresent` requires `NetError::Dns(DnsError::NoRecordsFound(no_records))` with `response_code == NoError` and `negative_ttl == Some(nonzero)`, and uses the minimum A deadline and checked `now + negative_ttl`. NXDOMAIN, missing/zero TTL, overflow, malformed answers, and every other response are errors. Automatic, static-prefix, and explicit-disabled modes follow the Task 3 state machine. The production transport owns one atomic `{ generation, valid_until, address_policy }` snapshot; every async snapshot read resamples the clock before rejecting `now >= total_deadline` or `now >= valid_until`. The first fetch at/after expiry refreshes single-flight before any user-controlled DNS. A failed expired refresh blocks the fetch. Same-policy TTL renewal may preserve generation while publishing a new deadline; policy changes increment generation.
- Public `#[async_trait::async_trait] trait FeedTransport: Send + Sync { async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError>; }` is the injection seam used by later domain/integration tests. Production `HttpFeedTransport` owns private async `DnsResolver` and per-hop async `HttpExecutor` seams; unit tests inject fakes without exposing a runtime loopback bypass.
- `FetchOutcome::Document` owns the final normalized response URL, bounded decoded body bytes, an optional single validated UTF-8 `Content-Type` string for Task 5 MIME/charset preflight, and optional response `OpaqueValidator` ETag/Last-Modified values. `FetchOutcome::NotModified` owns the final URL and optional response validators so a 304 can update metadata without polling a body. Multiple `Content-Type`, ETag, or Last-Modified fields and a non-UTF-8 Content-Type are typed response errors. Public accessors expose owned typed metadata; Debug prints only the redacted URL view, byte count, and field presence.
- Each hop builds a short-lived reqwest client with `redirect(Policy::none())`, `no_proxy()`, `no_gzip()`, `no_brotli()`, `no_deflate()`, `no_zstd()`, 5-second connect timeout, 10-second read-idle timeout, remaining hop/total deadlines, and `resolve_to_addrs(host, approved_socket_addrs)`.
- At most 16 DNS results and five redirects. Any denied address rejects the whole set. Every redirect repeats URL/DNS/address validation and never forwards validators across a changed validator URL.
- The 16-address limit applies to the raw resolver result before deduplication: zero or more than 16 rejects, otherwise exact duplicate `IpAddr`s are removed before socket construction. Five redirect responses are allowed after the initial request; a sixth redirect response rejects. A successful compressed ratio is inclusive at `decoded_len <= compressed_len * 100`; the next decoded byte rejects.
- A successful response must have `remote_addr()` in the approved address set. Missing/mismatched peer information rejects before body processing.
- Absolute deadlines cover DNS 3 s, connect 5 s, first byte 10 s, each body-idle interval 10 s, one HTTP request/redirect hop 20 s, and the whole refresh 30 s. A NAT64 discovery deadline maps to public `Timeout/Total` only after the total deadline; the RFC 7050-local three-second expiry remains `Nat64Discovery`. A new redirect target starts a new 20-second hop deadline, while DNS replays within that hop do not; no redirect or replay resets the 30-second total deadline.
- Automatic decompression is disabled. Compressed bytes are capped at 2 MiB; decoded bytes at 10 MiB; ratio at 100:1. `Content-Encoding` accepts only: no field (identity), or exactly one field whose OWS-trimmed bytes case-insensitively equal `identity`, `gzip`, `br`, or `deflate`. Empty values, multiple header fields, comma lists/layering, parameters, non-ASCII bytes, and every other token reject with a typed encoding error. HTTP `deflate` means zlib-wrapped DEFLATE in v1; raw DEFLATE returns a typed decode error. Gzip processes all members under the same decoded/ratio budgets.
- Status handling is exact: `200` yields the bounded decoded document; `304` yields `NotModified` without polling a body; only `301/302/303/307/308` redirect and require exactly one valid `Location`; every other status returns a typed status error without reading the body. If a single valid `Retry-After` is present on that status, the transport parses and preserves it relative to the response receipt time; multiple or invalid values are typed response errors.
- `FeedFetchError` strips reqwest URLs with `without_url()` before storing a source. Custom `Debug` output contains only error class, host, and counts.

- [ ] **Step 1: Write failing transport abuse tests**

Place abuse tests inside `resolver.rs`, `fetch.rs`, and `decode.rs` unit-test modules. `test_support.rs` is compiled only under `#[cfg(test)]` and contains fake resolver/executor/scripted server utilities. Separate integration-test processes cover conflicting rustls provider installation and the `--version` fast path. No integration test or production API receives a loopback-allowing policy.

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

### Task 5: Feed parsing, XML preflight, and HTML sanitization

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/feeds/parse/mod.rs`
- Create: `src/feeds/parse/types.rs`
- Create: `src/feeds/parse/mime.rs`
- Create: `src/feeds/parse/encoding.rs`
- Create: `src/feeds/parse/xml.rs`
- Create: `src/feeds/parse/json.rs`
- Create: `src/feeds/parse/map.rs`
- Create: `src/feeds/parse/finalize.rs`
- Create: `src/content/mod.rs`
- Create: `src/content/sanitize/mod.rs`
- Create: `src/content/sanitize/policy.rs`
- Create: `src/content/sanitize/images.rs`
- Create: `src/content/sanitize/hash.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/lib.rs`
- Create: `tests/fixtures/rss_2_60_items.xml`
- Create: `tests/fixtures/atom_mixed.xml`
- Create: `tests/fixtures/rss_1_rdf.xml`
- Create: `tests/fixtures/json_feed.json`
- Create: `tests/fixtures/rss_windows_1252.xml`
- Create: `tests/fixtures/rss_identity_edges.xml`
- Create: `tests/fixtures/malicious_html.xml`
- Create: `tests/fixtures/unsafe_xml.xml`
- Create: `tests/fixtures/feed_manifest.json`
- Create: `tests/feed_parse_sanitize.rs`

**Interfaces:**

- Keep `feedparser-rs = { version = "=0.5.5", default-features = false }` parser-only. Add exact direct pins `quick-xml = "=0.41.0"`, `ammonia = "=4.1.3"`, `encoding_rs = "=0.8.35"`, `html5ever = "=0.39.0"`, and `mime = "=0.3.17"`; each is already present in the locked graph. Never use `parse_url`, `FeedHttpClient`, `ParseOptions`, parser-owned HTTP, or feedparser-rs sanitizer helpers.
- `FetchedDocument::try_from(FetchOutcome)` accepts only `Document`, consumes the final `NormalizedFeedUrl`, bounded decoded body, Content-Type, ETag, and Last-Modified, and preserves them for later persistence. Its separate `FetchedDocumentError::NotDocument` rejects a 304 before the parser.
- `FeedParser::parse(FetchedDocument) -> Result<ParsedFeed, FeedParseError>` is async and uses one process-wide `Arc<Semaphore>` with exactly two permits. Capacity acquisition is fail-fast with `try_acquire_owned`: no queue of retained 10 MiB documents is allowed. The owned permit, body, URL, and validators move into one `spawn_blocking` closure and remain held through preflight, parser mapping, both sanitizer passes, final identity folding, and duplicate arbitration. Aborting the awaiter does not release capacity before the closure exits. Busy, closed semaphore, and worker panic are distinct typed errors.
- MIME is parsed strictly. Direct XML MIME is `application/rss+xml`, `application/atom+xml`, `application/rdf+xml`, `application/xml`, or `text/xml`; direct JSON MIME is `application/feed+json` or `application/json`. Missing MIME, `text/plain`, `text/html`, and `application/octet-stream` require decoded safe sniffing. `image/*`, `audio/*`, `video/*`, PDF, and every unsupported MIME reject even when their body mimics a feed. A direct XML/JSON MIME that disagrees with the decoded body returns `MimeMismatch`. Duplicate or malformed charset parameters reject.
- Encoding precedence is BOM, then HTTP charset, then a syntactically valid leading XML declaration, then UTF-8. Every explicit label is validated with `encoding_rs`; unknown labels and UTF-32 are typed rejects. Conversion is strict and may not emit U+FFFD. Both the transport body and converted UTF-8 are capped at 10 MiB. The selected original encoding is recorded separately because feedparser-rs will see normalized UTF-8.
- XML preflight accepts XML 1.0 only; a missing declaration defaults to 1.0 and every other declared version rejects. It fully consumes EOF with quick-xml checks enabled, permits at most depth 128, 1,000,000 non-EOF events, 256 attributes per element including namespace attributes, and 64 KiB raw value per attribute. It requires exactly one root, rejects malformed/truncated XML, duplicate attributes, any `DOCTYPE`, every named entity except the five XML predefined entities, and every general reference outside the root before or after it. Numeric references are accepted only for XML 1.0 legal characters: tab, LF, CR, `U+0020..=U+D7FF`, `U+E000..=U+FFFD`, or `U+10000..=U+10FFFF`. It removes only a leading BOM and the single leading XML declaration by byte offset, never reserializes, normalizes whitespace, reorders attributes, or rewrites CDATA/entity spelling.
- XML preflight also produces a feed/entry-indexed effective-base sidecar. Every explicit `xml:base` resolves against its parent effective base, starting from the final response URL, and must become credential-free HTTP(S); invalid/unsafe bases reject the document. Leaf `link`/`enclosure`-level `xml:base` that feedparser-rs ignores is rejected. Relative parser DTO URLs resolve with the validated feed/entry sidecar; a validated content/summary detail base may override it. This prevents provenance loss from becoming a silent response-URL fallback.
- Before feedparser-rs, preflight validates exact feed signatures: RSS `rss` roots require an explicit supported version; Atom roots require the exact Atom 0.3 or 1.0 namespace; RDF requires the RDF root plus an RSS 1.0 declaration whose attribute name is exactly `xmlns` or starts with `xmlns:` (ordinary names such as `xmlnsfoo` never count); JSON requires the exact JSON Feed 1.0/1.1 version URI. A feed-shaped root with a missing/unknown version is `UnsupportedVersion`; a well-formed non-feed root under direct XML/JSON MIME is `MimeMismatch`.
- JSON Feed receives a strict structural precheck before feedparser-rs: top-level object, exact version, at most 128 nested array/object containers (scalar leaves do not add a level), at most 5,000 items, at most 64 attachments per item, and N/N+1 string/collection checks for every persisted field so upstream silent truncation cannot become accepted data. Every attachment requires present string `url` and `mime_type`; optional attachment fields reject null/wrong types. The original bounded document is depth-checked before serde parsing, and the upstream parser receives only the already-validated JSON Feed fields, so an ignored deep extension cannot trip feedparser-rs's lower internal serde recursion boundary; exactly 128 containers pass and 129 reject.
- Before `feedparser_rs::parse_with_limits`, enforce `MAX_PROJECTED_INHERITANCE_BYTES = 32 * 1024 * 1024` per document with checked multiplication/addition; exactly 32 MiB passes, anything larger or any arithmetic overflow returns typed `ProjectedInheritanceTooLarge` without invoking feedparser-rs. Freeze `PERSON_STRUCT_BYTES = size_of::<feedparser_rs::Person>()` and `SMALL_STRING_STRUCT_BYTES = size_of::<feedparser_rs::types::SmallString>()` as runtime/platform structural costs. Every cloned author vector costs `author_count * PERSON_STRUCT_BYTES` plus all cloned `name`, `email`, `uri`, and `avatar` UTF-8 payload bytes. JSON item author inheritance additionally costs one `SMALL_STRING_STRUCT_BYTES + first.name bytes` flat-author clone when the first name is present and one `PERSON_STRUCT_BYTES + first Person payload` `author_detail` clone; nonempty `authors` or an effective legacy `author` suppresses inheritance, while `authors: []` inherits and suppresses the legacy object. Atom costs one document-level feed-author vector clone plus the same vector clone for every entry without parsed authors and a `SMALL_STRING_STRUCT_BYTES + final feed flat-author bytes` clone when that flat author exists; author sources include standard Atom `author`, canonical/custom-prefix Dublin Core `creator`, and canonical/custom-prefix iTunes `author`. Each inherited language destination costs `SMALL_STRING_STRUCT_BYTES + language UTF-8 payload bytes`: JSON counts `Entry.language` plus every present `title`, `content_html`, `content_text`, and `summary`; item language absent or empty inherits. Atom counts each recognized entry `title`, `subtitle`/`tagline`, `rights`/`copyright`, `summary`, and retained `content` clone; feed/entry/construct `xml:lang` or bare `lang` overrides inheritance and an empty value clears it, while self-closing content counts only when `src` is present. Empty/short author payloads still pay structural `Person` clone cost.
- Build all 23 `ParserLimits` fields explicitly rather than inheriting `strict()`: entries 5,000; links/feed 256; links/entry 256; authors 256; contributors 256; tags 256; content blocks 65 sentinel; enclosures 65 sentinel; namespaces 256; nesting 128; text `1 MiB + 1` sentinel; feed bytes 10 MiB; attributes 64 KiB; and each Podcast 2.0 collection field 256. JSON content, attachments, links, tags, and authors are independently prechecked because upstream incorrectly applies `max_entries` to those collections. Domain caps remain 64 content blocks/enclosures, 64 KiB title, 1 MiB final sanitized HTML, 16 MiB total normalized entry text, and 256 KiB canonical enclosure JSON. Total normalized entry text sums UTF-8 bytes for GUID, canonical URL, title, every author, summary, selected content blocks, final HTML, and every enclosure string; inert-image metadata uses its separate 256 KiB cap. `Unknown`, `bozo=true`, or a nonempty bozo exception always rejects; exception text is never exposed.
- Supported formats are RSS 0.90, RSS 0.91 Userland without DTD, RSS 0.92, RSS 1.0/RDF, RSS 2.0, Atom 0.3/1.0, and JSON Feed 1.0/1.1. DTD-bearing RSS 0.91 Netscape is deliberately unsupported and returns `DoctypeForbidden`.
- Parser types are immediately copied into owned `ParsedEntryCandidate` values. Content blocks take precedence over summary; multiple blocks join with a literal `\n`; plain text is HTML-escaped before sanitization. Display date is `published.or(updated)`; duplicate arbitration time is `updated.or(published)`; parsing never reads the current time.
- Every feed, entry, anchor, image, and enclosure URL is revalidated as credential-free HTTP(S), normalized with fragments removed, and resolved against the validated effective base. Protocol-relative URLs are accepted only after resolution against a valid HTTP(S) base.
- `sanitize_entry_html` uses this exact allowed-tag set: `a, abbr, b, blockquote, br, caption, code, col, colgroup, dd, del, details, div, dl, dt, em, figcaption, figure, h1, h2, h3, h4, h5, h6, hr, i, img, kbd, li, mark, ol, p, pre, q, rp, rt, ruby, s, samp, small, span, strong, sub, summary, sup, table, tbody, td, tfoot, th, thead, tr, u, ul, var`. Generic attributes are empty; `a` allows only `href`; final `img` allows only `alt,width,height`; `td/th` allow canonical `colspan,rowspan` in `1..=100`. Clean-content tags are `base, embed, form, frame, frameset, head, iframe, link, math, meta, noscript, object, script, style, svg, template`. URL schemes are only HTTP(S), and anchor rel is fixed to `noopener noreferrer nofollow`.
- The sanitizer removes every event/style/class/id/publisher-data/tracking/fetch-capable attribute. Images retain alt and bounded positive width/height in final HTML but no `src`, `srcset`, empty source, poster, background, or SVG href. Final HTML is at most 1 MiB. Each entry has at most 256 images; alt is at most 4 KiB; retained width/height are canonical integers in `1..=16_384`; total canonical inert metadata is at most 256 KiB. Safe original image URLs are out-of-band `InertImage { image_index, ... }` records, where `image_index` is the zero-based retained `img` ordinal rebuilt and validated against the second/final sanitizer output. Unsafe/source-less images have no record and cannot shift later records because matching is by explicit index. Publisher URLs are never hidden in `data-*` HTML attributes.
- Hashes are frozen. `source_document_hash` is ordinary BLAKE3 over the exact transport-decoded bytes before charset conversion. Both semantic hashes use exact frame `b"RDHC\0\x01" || 0x01 || u32_be(html_utf8_len) || exact_html_utf8`. `source_content_hash` derives with context `raindrop.entry-source-content.v1` over core-sanitized HTML; after the future content-processing insertion point, `content_hash` derives with `raindrop.entry-content.v1` over final HTML. Inert image source URLs are excluded; alt/width/height remain in HTML. With no plugins the framed payloads are equal, while domain-separated digests intentionally differ. An independent golden vector freezes the frame apart from sanitizer fixtures.
- The internal order is fixed as: `FetchedDocument -> MIME/encoding/preflight/parser -> ParsedEntryCandidate -> core sanitize/normalize -> source_content_hash -> content-processing insertion point -> second sanitize/normalize -> content_hash -> host-owned EntryIdentity -> duplicate arbitration -> ParsedEntry`. Task 5 uses a concrete no-processor path, not an empty public plugin trait. New Task 5 candidates, finalizer inputs, document index, and arbitration time remain `pub(crate)`; existing public Task 3 `StableEntryFields` and `EntryIdentity::from_parts` remain grandfathered. Plugins will never be allowed to set GUID, canonical URL, identity kind/hash, or database ID.
- Duplicate folding groups by identity index hash and then compares identity kind plus full identity. A hash match with different full identity returns `IdentityHashCollision`. Newer arbitration time wins; `Some` wins over `None`; ties retain the first document slot even when its value is replaced.
- `FeedParseError` is layered into content-type, encoding, XML, parser, limit, normalize, sanitize, identity, capacity, and worker categories. Stable limits include `SanitizedContentTooLong`, `TooManyImages`, `ImageAltTooLong`, `ImageDimensionInvalid`, and `ImageMetadataTooLarge`. Public Debug/Display exposes only stable category, format, count, byte length, and hash presence; it never includes body text, publisher HTML, bozo exception, validator, or full URL/query.

- [ ] **Step 1: Add deterministic fixtures and failing golden tests**

The 60-item fixture is synthetic, not a copy of current IT Home content. It contains fixed timestamps, unique GUIDs, escaped HTML descriptions, relative/absolute links, remote images, and tracking attributes. The versioned manifest groups expectations by file and records ordered identity kind/full value/index hash/content hash. Raw hash is ordinary BLAKE3 over exact fixture bytes; decoded hash is test-only ordinary BLAKE3 over the strict UTF-8 string before leading BOM/XML-declaration removal. The Windows-1252 fixture is binary and has a fixed raw BLAKE3 so editors cannot silently rewrite it.

Required tests:

```text
RSS 2.0 fixture parses exactly 60 stable identities
Atom xml:base and published/updated map correctly
RSS 1.0/RDF and JSON Feed fixtures map through the same owned domain contract
RSS 0.91 Userland passes while DTD-bearing Netscape rejects
BOM, HTTP charset, and XML declaration precedence decode once; unknown, duplicate, malformed, UTF-32, lossy, and expanded-over-limit encodings reject
missing/tolerated MIME requires sniff; direct MIME mismatch and hard-deny MIME reject even with a feed-shaped body
preflight preserves parser bytes exactly while allowing predefined/numeric references and rejecting custom entities, DTD, malformed attributes, multiple roots, deep/event/attribute/value limits, and truncation
JSON and XML N/N+1 sentinels prove 1 MiB/1 MiB+1 text, 64/65 enclosures, 64/65 content blocks, and 5,000/5,001 entries cannot be silently truncated
duplicate/missing GUIDs follow deterministic identity precedence; hash collision rejects; newer updated wins and ties keep the first slot
plain text containing markup is escaped, while malicious HTML loses scripts/events/styles/forms/frames/SVG/MathML/unsafe URLs
the sanitized DOM has no fetch-capable attribute and every inert image record keeps the correctly paired safe URL/alt/width/height out of band
tracking/style/image-source-only changes keep the same semantic hash while real content changes do not
single-title, total-normalized-content, image-count/metadata, enclosure-count, and canonical enclosure-JSON budgets reject with typed errors
two started blocking closures keep both permits after awaiter abort; a third call returns ParserBusy until those closures really exit
many concurrent calls fail fast and do not form an unbounded retained-body queue
all parse errors redact full URL query, body, HTML, validator, and parser exception text
```

- [ ] **Step 2: Add compiling parser/sanitizer interfaces**

Define `FetchedDocument`, `ParsedSource`, `ParsedFeed`, `ParsedEntry`, `ParsedEnclosure`, `SanitizedContent`, `InertImage`, `ParsedFeedVersion`, and typed preflight/parse/sanitize errors. Crate types do not cross the public seam.

- [ ] **Step 3: Implement one corpus behavior at a time**

Implement focused RED/GREEN slices in this order: typed interfaces; MIME/strict charset; XML preflight; JSON precheck; feedparser limits/bozo/version mapping; RSS 2.0; Atom/xml:base; RSS 1.0/RDF; JSON Feed; owned field and enclosure budgets; plain-text escaping; sanitizer allowlist; inert images; semantic hashes; final identity folding; collision-safe duplicate arbitration; semaphore cancellation/fail-fast behavior; redaction.

- [ ] **Step 4: Verify corpus and dependency graph**

```bash
cargo tree --locked -p feedparser-rs@0.5.5 -e features
! cargo tree --locked -e features | rg 'feedparser-rs feature "(default|http)"|reqwest feature "blocking"'
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_parse_sanitize -- --nocapture --test-threads=1
cargo +1.94.0 test --locked --test feed_parse_sanitize -- --nocapture --test-threads=1
cargo test --locked --all-features
```

Expected: PASS; exactly one quick-xml 0.41, ammonia 4.1.3, encoding_rs 0.8.35, html5ever 0.39.0, and mime 0.3.17 node is shared by direct Raindrop use and the existing graph. feedparser-rs has no `http`/reqwest feature path.

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

- Create: `src/db/migration/rss/entry_storage.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Create: `src/feeds/content_storage.rs`
- Create: `src/feeds/persistence.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/feeds/repository.rs`
- Modify: `src/feeds/refresh.rs`
- Modify: `tests/support/database.rs`
- Modify: `tests/rss_migrations.rs`
- Create: `tests/feed_entry_persistence.rs`

**Interfaces:**

- `sanitized_content` stores exact prefix `rdsc:v1:` followed by compact canonical JSON `{"html":...,"inertImages":[...]}`. The private storage structs declare fields in that exact order; inert images are strictly increasing by `imageIndex`, have unique indexes, and retain only sanitized absolute source URLs plus bounded alt/width/height metadata. Encoding and decoding reject envelopes above 4 MiB, unknown prefixes/versions, malformed JSON, duplicate/out-of-order indexes, and invalid dimensions with typed redacted errors. No raw publisher HTML is accepted at this seam.
- `PersistedContent::from_sanitized(&SanitizedContent)` owns the canonical envelope; `PersistedContent::decode(&str)` produces the typed detail shape `{ html, inert_images }` used later by Task 9. Serialization names are `html` and `inertImages`; image names are `imageIndex`, `sourceUrl`, `alt`, `width`, and `height`.
- The storage envelope never participates in entry identity or semantic hashes. `source_content_hash` and `content_hash` are the Task 5 HTML-only values; an image-source-only envelope change therefore does not create a new entry and does not trigger a large-field rewrite.
- Add an ordered schema-evolution migration after all existing migrations. On MySQL it widens `entries.sanitized_content` to `LONGTEXT` and `identity/title/author/summary/enclosure_json` to `MEDIUMTEXT`; SQLite and PostgreSQL remain logical `TEXT`. The migration is idempotent and its down path is verified only with bounded fixture data. Fresh databases and already-migrated databases converge to the same physical schema.
- A persist transaction first verifies database-clock owner/token fencing, then compares existing identities, increments `INGEST_GENERATION` only when at least one new entry exists, allocates monotonic feed sequences, updates changed content, updates Feed/run state, and releases the lease.
- `(feed_id, identity_hash)` is final duplicate arbitration; a hit compares both persisted `identity_kind` and full `identity`, and returns `IdentityHashCollision` if either differs.
- Existing rows preserve ID, inserted time, ingest generation, feed sequence, and sort key. Tracking-only sanitized equality avoids a large-field rewrite. `sort_at_us` is derived from checked `published_at_us` or insertion time once and never changes.
- `PersistFeed` owns the exact final validator URL plus optional `OpaqueValidator` ETag/Last-Modified values. The same feed transaction encodes them to canonical storage text; repository readback decodes them before the next fetch. Corrupt/unknown validator storage is a typed repository error and is never forwarded as a header.
- Keep `FeedRepository` as the public repository type, but put Task 7 entry persistence SQL and methods in `src/feeds/persistence.rs` via a separate `impl FeedRepository`; expose its database field only as `pub(super)`. Do not grow the refresh-claim SQL file with entry upsert logic.
- Backend-specific exact unique-conflict/upsert functions are private. Broad MySQL `INSERT IGNORE` is forbidden. Transaction retry reuses parsed content and never repeats network I/O.

- [ ] **Step 1: Add the storage envelope and schema-evolution contract**

Write RED/GREEN tests for deterministic `rdsc:v1:` bytes, round-trip of HTML plus indexed inert images, rejection of corrupt/oversized/unknown envelopes, image-source-only changes preserving the existing HTML hashes, and MySQL physical column types. Extend `rss_migrations` so the new migration participates in up/down/up and named schema verification.

- [ ] **Step 2: Add one compiling persist input/output seam**

Define owned `PersistFeed`, `PersistEntry`, and `PersistResult` types. Begin with one new-entry test that compiles and fails on count/row assertions.

- [ ] **Step 3: Implement behavioral slices**

Run RED/GREEN cycles for: first 60 inserts; second identical refresh inserts zero; tracking-only change updates zero; a real content change under GUID/URL or unchanged fingerprint inputs updates one without changing identity fields/state; accepted content-only/title/date/enclosure fallback changes insert a new identity; concurrent persists leave one identity row; both kind/text are checked on hash collision; non-UTF-8 validator bytes survive database store/reload/decode on all three backends; all new rows share one generation and monotonic sequences; no-new-entry refresh does not increment the counter; a stale token writes nothing.

- [ ] **Step 4: Verify persistence**

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test feed_entry_persistence -- --nocapture
RAINDROP_TEST_POSTGRES_URL="$POSTGRES_TEST_URL" cargo test --locked --test feed_entry_persistence postgres -- --nocapture
RAINDROP_TEST_MYSQL_URL="$MYSQL_TEST_URL" cargo test --locked --test feed_entry_persistence mysql -- --nocapture
cargo test --locked --test rss_migrations -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add src/db src/feeds tests/support/database.rs tests/rss_migrations.rs tests/feed_entry_persistence.rs
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
- The IT Home smoke is `#[ignore]`, also checks `RAINDROP_LIVE_RSS_SMOKE=1`, uses a temporary SQLite database and at most two requests to `https://www.ithome.com/rss/`, passes the original response through the production charset conversion/preflight/feedparser-rs path without ad hoc whitespace or structural rewriting, and never fetches article pages/images.
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

`docs/rss-security.md` records scheme policy, SSRF address classes, DNS pinning, redirect rules, body/time limits, XML preflight, sanitizer policy, error redaction, the exact opt-in smoke command, and two existing audit exceptions. `RUSTSEC-2023-0071` is limited to sqlx-mysql's client-side RSA public-key encryption path and does not perform the private-key decryption targeted by Marvin; `RUSTSEC-2026-0173` is an unmaintained build-time SeaORM proc-macro dependency already tracked in `tasks/todo.md`. Both exceptions name their inverse dependency path and must be removed or re-reviewed on every SeaORM/sqlx upgrade. Mark only completed RSS checklist items; do not mark scheduling/API/Reader work that this plan has not delivered.

- [ ] **Step 5: Run the complete data-ingestion gate**

```bash
cargo install cargo-audit --locked --version 0.22.2
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo tree --locked -e features -i reqwest
! cargo tree --locked -e features | rg 'system-proxy|cookies|native-tls|http3'
cargo tree --locked -i rsa
cargo tree --locked -i proc-macro-error2
cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0173
git diff --check
```

Expected: every command exits 0; the two ignored advisories match only the documented existing sqlx-mysql and SeaORM macro paths, and the feedparser-rs subtree introduces no ignored advisory. Run the IT Home smoke separately and record its date/count/304 behavior without copying Feed content into the repository.

- [ ] **Step 6: Commit**

```bash
git add src/feeds tests/feed_ingestion_e2e.rs tests/live_rss_ithome.rs tasks/todo.md docs/rss-security.md
git commit -m "test: verify rss ingestion end to end"
```

## Plan self-review

- Spec coverage: this plan covers portable Feed/Subscription/Entry/EntryState storage, three-database schema contracts, safe URL fetching, conditional requests, parsing, sanitization, idempotent insertion, fenced refresh persistence, lifecycle outbox records, minimal production subscribe/list/detail domain seams, deterministic fixtures, and the required IT Home live smoke. Subscription HTTP routes, scheduler workers, categories, retention, OPML, and Reader UI intentionally remain in later independently runnable plans.
- DDIA: shared records, sparse user state, stable snapshot generation, feed sequence, unique constraints, fencing token, short transactions, database-as-record-system, and at-least-once outbox semantics are explicit.
- Security: every external boundary has limits and abuse tests; automatic proxy/redirect/decompression are disabled; DNS is pinned; custom NAT64 is discovered through a trusted RFC 7050 path; XML and HTML are treated as hostile; feed-domain/transport logs and errors do not expose URL query, validators, body, or secrets, and ORM models never cross the logging boundary.
- Type consistency: Task 1 produces entities verified across backends by Task 2; Task 3 produces URL/identity/schedule types consumed by Task 4 and later repositories; Task 4 produces bounded documents consumed by Task 5; Task 5 produces validated parsed content consumed by Tasks 7 and 9; Task 6 produces fenced refresh claims; Task 7 persists entries/generations; Task 8 records lifecycle events; Task 9 composes the stable transport/domain/query interfaces.
- Unresolved-marker scan: clean; later subsystems are named as explicit exclusions.
