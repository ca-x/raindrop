# Feed Retention v1 Design

## Objective

Implement bounded physical retention for orphaned Feeds so that subscriptions can be removed without leaving unbounded Feed, Entry, and refresh-run storage forever. The cleanup must never remove a currently subscribed Feed, must not race a new subscription into an internal error, and must preserve lifecycle outbox records for future plugin delivery and audit.

Success means an operator can configure an orphan grace period, the feed runtime periodically deletes only eligible orphaned Feeds, SQLite/PostgreSQL/MySQL share the same contract, and concurrent cleanup is idempotent across multiple Raindrop instances.

## Product and data contract

- A Feed becomes an orphan only when the last Subscription is deleted; the existing `feeds.orphaned_at` remains the authoritative retention timestamp.
- Default orphan grace is 30 days. `0` disables physical cleanup. Valid configured values are `0..=3650` days.
- Cleanup deletes only a Feed whose `orphaned_at` is not null and is no later than the database-clock cutoff, with no current Subscription and no `QUEUED` or `RUNNING` refresh run.
- Deleting the Feed intentionally uses existing foreign-key cascades to delete its Entries, EntryStates, and refresh runs.
- `lifecycle_outbox` intentionally has no refresh-run foreign key and survives Feed deletion. Future plugin delivery must be able to consume the committed payload without loading the deleted Feed.
- Retention scans at most 100 candidates per maintenance pass and processes candidates in `(orphaned_at, id)` order.
- The scheduler lane runs the retention pass at startup and then no more frequently than once per hour. Failures are logged and retried later; they do not stop feed fetching.
- The cleanup command is internal only. v1 adds no HTTP endpoint or administrator UI.

## Concurrency and consistency

The database is the source of truth. Candidate scans are advisory; every candidate is handled in a short transaction that locks the Feed row and then rechecks the cutoff, Subscription absence, and active-run absence before deletion.

Subscription creation and retention use the same Feed-row lock boundary. A subscription request may discover a Feed immediately before another instance deletes it. The subscription command therefore treats a missing pre-scanned Feed as a retryable race, restarts discovery, and either locks the surviving Feed or creates a new Feed with the same normalized URL. This retry is internal and never becomes a new public API error.

Two retention workers may scan the same candidate. The first deletion wins; the second observes the missing row and succeeds with no deletion. No in-memory lock is part of correctness.

## DDIA internal review

- Reliability: deletion predicates are rechecked under the authoritative row lock; a failed pass is retried and does not fail the runtime.
- Scalability: the selector uses a dedicated `(orphaned_at, id)` index and a fixed batch of 100 rather than an unbounded table scan or transaction.
- Maintainability: retention is a focused module, configuration is additive, and no second Feed state machine is introduced.
- Transaction safety: all destructive decisions occur after the lock in the same transaction, preventing write skew between Subscription insertion and Feed deletion.
- Multi-instance behavior: database locks and idempotent missing-row handling provide correctness; process identity and timing do not.
- Schema evolution: the migration only adds an index and is safe to re-enter; existing rows already contain the required timestamp.
- Derived data: lifecycle outbox is retained as an independent durable record and is not coupled back to the mutable Feed row.

## Tech stack and commands

- Rust 1.94.0, edition 2024, Axum, SeaORM/SeaQuery, Tokio, `time`.
- SQLite is always tested; PostgreSQL and MySQL contracts use the existing CI service URLs.
- No Rust or npm dependency is added.

Commands:

```bash
cargo fmt --check
cargo test --locked --test config_loading
cargo test --locked --test rss_migrations sqlite_rss_schema_contract
cargo test --locked --test feed_retention_contracts
cargo test --locked --test feed_runtime
cargo test --locked --all-features
git diff --check
```

## Project structure

- `src/config/`: additive operator configuration and validation.
- `src/db/migration/rss/retention.rs`: retention selector index only.
- `src/feeds/retention.rs`: retention policy, database-clock cutoff, candidate scan, locked recheck, and deletion.
- `src/feeds/subscription.rs`: retryable discovery/delete race handling.
- `src/feeds/runtime.rs`: scheduler-lane maintenance wiring.
- `tests/feed_retention_contracts.rs`: portable repository and multi-instance contracts.
- `tests/config_loading.rs`, `tests/rss_migrations.rs`, `tests/feed_runtime.rs`: boundary and integration coverage.

## Code style

Retention exposes one narrow repository command and one immutable policy value:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedRetentionPolicy {
    pub orphan_grace: Option<std::time::Duration>,
}

impl FeedRepository {
    pub async fn purge_orphaned_feeds(
        &self,
        grace: std::time::Duration,
        limit: u16,
    ) -> Result<usize, FeedRetentionError>;
}
```

Errors use typed enums with redacted `Debug`; SQL values are bound parameters; backend-specific SQL is isolated in statement builders.

## Testing strategy

- Configuration tests cover default, TOML/environment precedence, disabled `0`, maximum `3650`, invalid text, and overflow.
- Migration contracts require `idx_feeds_orphan_retention` on all three databases and verify migration re-entry.
- Repository contracts prove eligible cascade deletion, outbox survival, preservation predicates, batch bounds, repeat idempotency, and two-instance convergence.
- A deterministic race seam pauses subscription creation after Feed discovery, lets retention delete the Feed, then proves subscription retry creates one live Feed and one Subscription.
- Runtime integration starts with an old orphan Feed, observes deletion without a queued refresh, and shuts down cleanly.

## Boundaries

- Always: use database time, bind SQL values, recheck after locking, preserve outbox, run the three-database CI contracts, commit and push each completed task.
- Main-Agent decision: schema/index changes and runtime configuration are authorized by the user's request for autonomous implementation and DDIA internal review.
- Never: delete subscribed Feeds, delete outbox as part of this slice, add an HTTP retention endpoint, add dependencies, modify `.superpowers/research/`, or modify root `node_modules/`.

## Success criteria

1. Default configuration exposes a 30-day orphan grace; `0..=3650` is accepted and other values fail closed without echoing input secrets.
2. The migration adds `idx_feeds_orphan_retention(orphaned_at, id)` on SQLite, PostgreSQL, and MySQL.
3. Only old, unsubscribed, inactive orphan Feeds are deleted, in batches of at most 100.
4. Feed deletion cascades Feed-owned rows while lifecycle outbox rows remain queryable.
5. Two instances can run cleanup concurrently without errors or double-counting.
6. A concurrent resubscribe either preserves the locked Feed or retries and creates a replacement; it never returns an internal error caused by retention.
7. Runtime maintenance executes the policy without blocking or terminating refresh lanes.
8. Full Rust tests, formatting, diff checks, branch CI, commit, and push succeed.

## Out of scope

- Entry-level age/count pruning for subscribed Feeds.
- Refresh-run history pruning independent of Feed deletion.
- Lifecycle outbox/audit retention.
- Backup compaction or SQLite `VACUUM`.
- Administrator retention UI or manual cleanup endpoint.
- OPML, AI artifacts, plugin storage, and MCP retention.

## Open questions

None. The main Agent resolved the bounded v1 defaults through the DDIA review above, as requested by the user.
