# Feed Retention v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- Additive `RAINDROP_FEED_ORPHAN_RETENTION_DAYS` / `feed_orphan_retention_days` configuration with a 30-day default, `0` disable behavior, and strict `0..=3650` validation.
- Portable `idx_feeds_orphan_retention(orphaned_at, id)` migration and re-entry contract for SQLite, PostgreSQL, and MySQL.
- Bounded `FeedRepository::purge_orphaned_feeds` maintenance using the database clock, at most 100 ordered candidates, per-Feed transactions, row locking, and post-lock predicate rechecks.
- Physical deletion only for old orphan Feeds with no Subscription and no `QUEUED`/`RUNNING` refresh run.
- Existing foreign-key cascades remove Feed-owned Entries, EntryStates, and refresh runs, while independent lifecycle outbox records remain durable and queryable.
- Multi-instance cleanup convergence and an internal subscription discovery retry when retention deletes a Feed between URL-hash discovery and row locking.
- Scheduler-lane runtime integration that runs once after database readiness and then hourly; failures are logged and retried without terminating feed refresh lanes.
- Operator documentation and `.env.example` coverage.

## Delivery commits

- `52d1f13 feat: configure feed retention`
- `13cd63b refactor: keep config constructor bounded`
- `0836ea4 feat: purge orphaned feeds`
- `88e1e5c feat: run feed retention maintenance`

## DDIA review outcome

- Database rows and constraints remain the source of truth; no process-local lock participates in correctness.
- Candidate scans are advisory and bounded. Every destructive decision is repeated after the Feed lock in the same short transaction.
- The `(orphaned_at, id)` index and fixed 100-candidate batch bound scan and lock load.
- Missing rows are idempotent for competing retention workers.
- Subscription creation treats a missing pre-scanned Feed as a retryable race, preventing a retention timing window from becoming an internal API error.
- Outbox records remain an independently retained derived log, so future plugin delivery does not depend on mutable Feed or refresh-run rows.

## Deterministic local verification

- `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo test --locked --test config_loading`: 10 passed.
- `cargo test --locked --test rss_migrations sqlite_rss_schema_contract -- --nocapture`: passed.
- `cargo test --locked --test feed_retention_contracts -- --nocapture --test-threads=1`: 7 passed locally; PostgreSQL/MySQL tests are environment-gated in the focused command.
- `cargo test --locked --test feed_subscription_contracts sqlite -- --nocapture --test-threads=1`: 21 passed.
- `cargo test --locked --test feed_runtime runtime_retention_deletes_old_orphan_without_refresh_work -- --nocapture`: passed.
- `cargo test --locked --bin raindrop`: 4 passed.
- `cargo test --locked --all-features`: 438 tests discovered; 437 passed and the opt-in IT之家 live RSS smoke was ignored.
- `git diff --check`: passed.

## Remote CI evidence

GitHub Actions run `29638020707` at commit `88e1e5c482f1e046b4cece0b1170e28f1b9fae0f` completed successfully and is the authoritative remote verification run for this slice:

- Rust foundation passed formatting, Clippy, SQLite/PostgreSQL/MySQL migrations, retention contracts, refresh/runtime contracts, and the full Rust suite.
- ASTRYX Web, supply-chain audit, current-stable compatibility, and Windows durable-replacement compile gates passed.
- Release embedding passed the committed release gate and all 14 production Playwright scenarios.
- The real Dockerfile built successfully; the container ran as `10001:10001` and passed live health plus Docker health status.

## Operational contract

- Default cleanup grace is 30 days. `0` disables physical cleanup and allows orphan storage to grow without bound; the maximum configured grace is 3650 days.
- The scheduler lane runs cleanup immediately after the configured database becomes ready and then no more frequently than hourly.
- Each pass scans at most 100 candidates ordered by `(orphaned_at, id)` and may delete fewer when candidates become recent, subscribed, active, or already deleted by another instance.
- The process does not run SQLite `VACUUM` and does not prune entries belonging to subscribed Feeds, independent refresh history, lifecycle outbox, AI artifacts, plugin state, or backups.

## Existing advisories

- `proc-macro-error2 v2.0.1` remains the recorded future-incompatibility advisory through the SeaORM dependency chain.
- The IT之家 live RSS smoke remains opt-in behind `RAINDROP_LIVE_RSS_SMOKE=1`.

## Explicitly remaining

- CommaFeed follow-up: bulk mark-read snapshot, next unread source, and source-local search.
- Split queued/running refresh presentation and entry-level partial-failure feedback.
- Multi-user sorting/reading cursor completion, registration policy, administrator management, and OIDC.
- AI provider adapters, official AI plugin, lifecycle host, MCP client/server, OPML import/export, and broader backup/retention work.
