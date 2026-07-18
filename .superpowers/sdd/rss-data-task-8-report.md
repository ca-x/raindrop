# RSS Data Task 8 Report

## Status and scope

- Implemented transactional recording of feed refresh lifecycle events.
- This is record-only: no dispatcher, delivery worker, AI/plugin/MCP call, retry scheduler, or claim-delivery behavior was added.
- The migration is appended after `feed_metadata`; `refresh_id` intentionally has no foreign key.
- `aggregate_type` is fixed to `FEED`, and `aggregate_id` is the feed ID.

## Implementation

- Added the `lifecycle_outbox` entity and additive migration with the exact 16-column contract.
- Added the exact named indexes:
  - `uq_lifecycle_outbox_idem(idempotency_key)`
  - `uq_lifecycle_outbox_order(refresh_id,event_sequence)`
  - `idx_lifecycle_outbox_due(status,available_at,lease_until,id)`
- Added a focused `feeds::lifecycle` module. Typed serde structs emit compact deterministic v1 payloads in the required field order and enforce a 64 KiB UTF-8 byte limit.
- Event IDs are UUIDs per insert attempt. `available_at` and `created_at` use the backend database clock.
- `persist_feed` records sequence 10 `feed.refresh.persisted`, then sequence 20 `feed.refresh.completed`, inside the existing entries/feed/run/lease transaction. An event error rolls the whole transaction back.
- `complete_not_modified`, owned `complete_failure`, and the legacy pre-persist `complete_success`/`complete_partial` seam record only sequence 20 completed events in their existing owned transaction.
- The legacy seam does not represent Task 7 entry persistence and never fabricates sequence 10. The only source of a two-event 200 lifecycle is `persist_feed`; future service code must not substitute the legacy seam for it.
- Authorization failure, stale lease/token, invalid transition, and `record_lease_lost` record no lifecycle event.
- Idempotency keys are exactly `refresh:{run_id}:persisted:v1` and `refresh:{run_id}:completed:v1`. Existing events are accepted only when all immutable event fields and exact payload bytes match. Semantic and order-key conflicts return `LifecycleEventConflict`.
- No broad MySQL `INSERT IGNORE` or duplicate-key swallowing is used. SQLite unique handling is limited to extended codes 1555/2067; unrelated constraint and trigger failures remain database errors.
- Failure payloads accept bounded stable error codes only (`[A-Z][A-Z0-9_]*`) and never contain retry timestamps or free text.

## TDD evidence

- First RED: `cargo test --locked --test feed_lifecycle_outbox sqlite_successful_200_records_persisted_before_completed -- --nocapture` failed with `no such table: lifecycle_outbox`.
- Migration GREEN exposed the next RED: the table existed but the success path recorded zero events.
- Success GREEN: the public `persist_feed` seam recorded exact canonical payload bytes in sequence 10 then 20.
- Expanded matrix GREEN: successful 200, partial 200, 304, owned error, legacy completed-only, stale authorization zero events, exact duplicate retry, semantic conflict, order conflict, payload bounds, and ordering.
- Rollback RED: an induced SQLite sequence-20 trigger failure was incorrectly classified as a semantic conflict because all primary code 19 constraints were treated as unique violations.
- Rollback GREEN: unique provenance was narrowed to SQLite 1555/2067; the trigger failure returned `Database` and rolled back entries, generation, metadata, run, lease, and both events.

## Migration/schema contract

- Fresh migration, repeated migration, down/up, and deleted-marker reentry are covered.
- SQLite and the conditional PostgreSQL/MySQL contract verify:
  - exact column order, type family/length, nullability, and defaults;
  - `PENDING`/zero operational defaults and four operational timestamps;
  - exact index names, uniqueness, and column order;
  - no outbox foreign key and successful insertion of an orphan `refresh_id`;
  - recovery of a missing due index on migration reentry.

## Verification

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --locked --test feed_lifecycle_outbox -- --nocapture --test-threads=1`: PASS, 5/5. PostgreSQL/MySQL tests explicitly skipped because their URLs were not configured.
- `cargo +1.94.0 test --locked --test feed_lifecycle_outbox sqlite -- --nocapture --test-threads=1`: PASS, 3/3.
- `cargo test --locked --test rss_migrations -- --nocapture --test-threads=1`: PASS, 4/4; PostgreSQL/MySQL explicitly skipped without URLs.
- `cargo +1.94.0 test --locked --test rss_migrations sqlite -- --nocapture --test-threads=1`: PASS, 2/2.
- `cargo test --locked --test feed_entry_persistence sqlite -- --nocapture --test-threads=1`: PASS, 16/16.
- `cargo test --locked --test feed_refresh_claims sqlite -- --nocapture --test-threads=1`: PASS, 11/11.
- `env -u RAINDROP_TEST_POSTGRES_URL -u RAINDROP_TEST_MYSQL_URL cargo test --locked --all-features`: PASS, zero failures across the full Rust suite.
- `git diff --check`: PASS.

## CI and external database status

- CI now runs serial SQLite, PostgreSQL, and MySQL filters for `feed_lifecycle_outbox`, alongside the existing serial RSS migration/claim/persistence contracts.
- `RAINDROP_TEST_POSTGRES_URL` and `RAINDROP_TEST_MYSQL_URL` were absent locally. No local PostgreSQL or MySQL pass is claimed; those contracts were compile-checked and explicitly skipped, and CI owns their real execution.

## Files

- Added: `src/db/entities/lifecycle_outbox.rs`
- Added: `src/db/migration/rss/outbox.rs`
- Added: `src/feeds/lifecycle.rs`
- Added: `tests/feed_lifecycle_outbox.rs`
- Modified: `src/db/entities.rs`, `src/db/migration.rs`, `src/db/migration/rss/mod.rs`
- Modified: `src/feeds/mod.rs`, `src/feeds/persistence.rs`, `src/feeds/repository.rs`, `src/feeds/refresh.rs`
- Modified: `tests/rss_migrations.rs`, `.github/workflows/ci.yml`
- Added: `.superpowers/sdd/rss-data-task-8-report.md`

## Self-review and concerns

- Rechecked that event insertion happens before the surrounding transaction commits and that every error path drops or explicitly rolls back the transaction.
- Rechecked MySQL owned completion ordering: the feed row is locked first, then authorization uses a new `UTC_TIMESTAMP(6)` statement.
- Rechecked canonical payloads against exact byte literals; URL queries, validators, body/HTML, stack, owner, secret, retry timestamp, and free text are absent.
- Rechecked that mutable delivery state is not part of idempotency equality, while every immutable event field and exact payload byte is.
- No dispatcher behavior was introduced.
- The only local coverage limitation is the absent external database URLs described above. Cargo also reports the existing non-blocking `proc-macro-error2 v2.0.1` future-incompatibility warning.
- Review backlog (Minor): the SQLite `record_lease_lost` test uses an 80 ms sleep for a 40 ms lease; a future cleanup can replace it with direct expired database state or bounded polling to remove clock-dependent flakiness.
- The commit SHA is reported in the handoff because a commit cannot contain its own final hash.
