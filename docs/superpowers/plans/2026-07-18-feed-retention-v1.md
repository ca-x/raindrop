# Feed Retention v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans task-by-task. This repository is explicitly configured for inline main-Agent execution; do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add configurable, bounded, multi-instance-safe physical cleanup of orphaned Feeds while preserving lifecycle outbox records and retrying concurrent resubscription races.

**Architecture:** The existing `feeds.orphaned_at` timestamp remains the retention frontier. A dedicated retention repository module scans an indexed bounded candidate set, locks and rechecks each Feed in a short transaction, then relies on existing foreign-key cascades. The scheduler lane runs this maintenance with an additive configuration policy; subscription creation retries if a pre-scanned Feed disappears before its lock.

**Tech Stack:** Rust 1.94.0 edition 2024, SeaORM/SeaQuery, SQLite/PostgreSQL/MySQL, Tokio, `time`.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-feed-retention-v1-design.md` exactly.
- Do not add a Rust or npm dependency.
- Use database time for the retention cutoff.
- Default orphan retention is 30 days; `0` disables cleanup; valid values are `0..=3650`.
- Each pass scans and processes at most 100 candidates ordered by `(orphaned_at, id)`.
- Destructive predicates must be rechecked after locking the Feed row.
- `lifecycle_outbox` must survive Feed deletion.
- Do not modify `.superpowers/research/` or root `node_modules/`.
- Commit and push every completed task to `feature/foundation-bootstrap`.

---

### Task 1: Additive configuration and retention index

**Files:**
- Modify: `src/config/model.rs`
- Modify: `src/config/loader.rs`
- Modify: `tests/config_loading.rs`
- Create: `src/db/migration/rss/retention.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Modify: `tests/rss_migrations.rs`

**Interfaces:**
- Produces `RuntimeConfig::feed_retention() -> FeedRetentionConfig`.
- Produces `FeedRetentionConfig { orphan_grace: Option<std::time::Duration> }`.
- Produces named index `idx_feeds_orphan_retention(orphaned_at, id)`.

- [x] **Step 1: Write failing configuration contracts**

Add tests asserting the default is `Some(Duration::from_secs(30 * 86_400))`, TOML is overridden by `RAINDROP_FEED_ORPHAN_RETENTION_DAYS`, `0` maps to `None`, `3650` is accepted, and `3651`/non-numeric input return `ConfigError::InvalidValue` naming the environment variable.

- [x] **Step 2: Run the configuration RED gate**

Run:

```bash
cargo test --locked --test config_loading feed_orphan_retention -- --nocapture
```

Expected: compilation fails because `RuntimeConfig::feed_retention` does not exist.

- [x] **Step 3: Implement the additive configuration contract**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedRetentionConfig {
    pub orphan_grace: Option<std::time::Duration>,
}

impl RuntimeConfig {
    #[must_use]
    pub const fn feed_retention(&self) -> FeedRetentionConfig {
        self.feed_retention
    }
}
```

Parse `RAINDROP_FEED_ORPHAN_RETENTION_DAYS` before TOML `feed_orphan_retention_days`, default to `30`, reject values above `3650`, and map `0` to `None`.

- [x] **Step 4: Write the migration RED contract**

Require `idx_feeds_orphan_retention` in `assert_expected_indexes`, drop it in the backend-specific migration re-entry fixture, rerun `migrate`, and require it to be restored.

- [x] **Step 5: Run the migration RED gate**

Run:

```bash
cargo test --locked --test rss_migrations sqlite_rss_schema_contract -- --nocapture
```

Expected: FAIL with missing named RSS index `idx_feeds_orphan_retention`.

- [x] **Step 6: Add the idempotent index migration**

Create `CreateFeedRetention` whose `up` creates:

```rust
Index::create()
    .name("idx_feeds_orphan_retention")
    .table(Feeds::Table)
    .col(Feeds::OrphanedAt)
    .col(Feeds::Id)
    .if_not_exists()
```

Register it after feed metadata and before lifecycle outbox. `down` drops only this index when present.

- [x] **Step 7: Verify Task 1**

Run:

```bash
cargo fmt --check
cargo test --locked --test config_loading
cargo test --locked --test rss_migrations sqlite_rss_schema_contract -- --nocapture
git diff --check
```

Expected: all pass.

- [x] **Step 8: Commit and push**

```bash
git add src/config/model.rs src/config/loader.rs tests/config_loading.rs \
  src/db/migration/rss/retention.rs src/db/migration/rss/mod.rs src/db/migration.rs \
  tests/rss_migrations.rs docs/superpowers/specs/2026-07-18-feed-retention-v1-design.md \
  docs/superpowers/plans/2026-07-18-feed-retention-v1.md tasks/plan.md
git commit -m "feat: configure feed retention"
git push origin feature/foundation-bootstrap
```

### Task 2: Portable retention transaction and resubscribe race recovery

**Files:**
- Create: `src/feeds/retention.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/feeds/repository.rs`
- Modify: `src/feeds/subscription.rs`
- Create: `tests/feed_retention_contracts.rs`

**Interfaces:**
- Produces `FeedRetentionError` with `Database`, `InvalidRequest`, `InvalidTime`, and `CorruptData` variants plus redacted `Debug`.
- Produces `FeedRepository::purge_orphaned_feeds(grace: Duration, limit: u16) -> Result<usize, FeedRetentionError>`.
- Produces internal `try_lock_feed_for_queue(...) -> Result<Option<LockedFeed>, RefreshRepositoryError>` used by retention-safe subscription retry.

- [ ] **Step 1: Write failing portable retention contracts**

Create SQLite-always and environment-gated PostgreSQL/MySQL contracts that seed:

1. one old orphan eligible for deletion with Entries, terminal refresh runs, and lifecycle outbox;
2. one recent orphan;
3. one old Feed with a Subscription;
4. one old orphan with a `QUEUED` run;
5. more eligible orphans than the requested batch limit.

Assert eligible Feed-owned rows cascade, outbox survives, protected rows remain, a second pass returns zero, and the return count never exceeds `limit`.

- [ ] **Step 2: Add multi-instance and resubscribe race RED tests**

Run two repositories against the same database with `tokio::join!` and assert the summed deletion count equals the number of eligible Feeds. Add a debug-only subscription hook that pauses after URL-hash discovery; delete the discovered orphan; release subscription creation; assert it retries to one Feed and one Subscription without exposing an error.

- [ ] **Step 3: Run the repository RED gate**

Run:

```bash
cargo test --locked --test feed_retention_contracts -- --nocapture --test-threads=1
```

Expected: compilation fails because the retention repository API is missing.

- [ ] **Step 4: Implement optional Feed locking**

Refactor the existing lock into:

```rust
pub(super) async fn try_lock_feed_for_queue<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<LockedFeed>, RefreshRepositoryError>
where
    C: ConnectionTrait;
```

Keep `lock_feed_for_queue` as a wrapper that maps `None` to `CorruptData` for existing callers.

- [ ] **Step 5: Implement retryable subscription discovery**

Use a private attempt error:

```rust
enum SubscribeAttemptError {
    RetryDiscovery,
    Repository(RefreshRepositoryError),
}
```

When an existing candidate disappears before its Feed lock, roll back and restart the three-attempt discovery loop. Preserve existing unique-violation retries and public error mapping.

- [ ] **Step 6: Implement bounded retention**

In `src/feeds/retention.rs`:

1. validate non-zero grace and `1..=100` limit;
2. read the backend database clock and subtract the configured grace;
3. scan IDs with `orphaned_at <= cutoff ORDER BY orphaned_at, id LIMIT ?`;
4. for each ID, begin a transaction and optionally lock the Feed;
5. skip missing/recent/non-orphan rows;
6. skip when a Subscription or `QUEUED`/`RUNNING` run exists;
7. delete exactly the locked Feed row and commit;
8. count only successful deletions.

- [ ] **Step 7: Verify Task 2**

Run:

```bash
cargo fmt --check
cargo test --locked --test feed_retention_contracts -- --nocapture --test-threads=1
cargo test --locked --test feed_subscription_contracts sqlite -- --nocapture --test-threads=1
git diff --check
```

Expected: all pass.

- [ ] **Step 8: Commit and push**

```bash
git add src/feeds/retention.rs src/feeds/mod.rs src/feeds/repository.rs \
  src/feeds/subscription.rs tests/feed_retention_contracts.rs \
  docs/superpowers/plans/2026-07-18-feed-retention-v1.md
git commit -m "feat: purge orphaned feeds"
git push origin feature/foundation-bootstrap
```

### Task 3: Runtime maintenance, operator docs, and final verification

**Files:**
- Modify: `src/feeds/runtime.rs`
- Modify: `src/main.rs`
- Modify: `tests/feed_runtime.rs`
- Modify: `.env.example`
- Modify: `docs/configuration.md`
- Modify: `tasks/todo.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/feed-retention-v1-report.md`

**Interfaces:**
- Produces `FeedRuntime::with_retention_policy(FeedRetentionPolicy)` while preserving `FeedRuntime::new` defaults for current callers.
- Production consumes `RuntimeConfig::feed_retention()`.

- [ ] **Step 1: Write the runtime RED test**

Seed a configured database with an old orphan Feed and no work queue. Start `FeedRuntime` with a short debug scan interval and one-day grace, wait until the Feed disappears, then request shutdown and require clean completion.

- [ ] **Step 2: Run the runtime RED gate**

Run:

```bash
cargo test --locked --test feed_runtime retention -- --nocapture
```

Expected: compilation fails because runtime retention policy wiring is missing.

- [ ] **Step 3: Wire scheduler-lane maintenance**

Add `FeedRetentionPolicy` to the runtime with defaults of 30-day grace, one-hour scan interval, and batch limit 100. Scheduler lane 0 runs retention immediately and then at the interval. `None` disables the deletion call. Log `deleted` at info when non-zero and log typed failures at warn without terminating the lane.

- [ ] **Step 4: Wire production configuration**

Read `loaded.runtime.feed_retention()` before moving other runtime fields and pass it to `production_feed_runtime`. Setup-required mode remains inert because `FeedRuntime::run` still waits for a ready database before constructing lanes.

- [ ] **Step 5: Document the operator contract**

Add `RAINDROP_FEED_ORPHAN_RETENTION_DAYS` and TOML `feed_orphan_retention_days` to configuration docs and `.env.example`, including default `30`, `0` disable behavior, maximum `3650`, one-hour bounded maintenance, cascaded Feed-owned data, and preserved lifecycle outbox.

- [ ] **Step 6: Run final bounded gates**

```bash
cargo fmt --check
cargo test --locked --test config_loading
cargo test --locked --test rss_migrations sqlite_rss_schema_contract -- --nocapture
cargo test --locked --test feed_retention_contracts -- --nocapture --test-threads=1
cargo test --locked --test feed_runtime retention -- --nocapture
cargo test --locked --all-features
git diff --check
```

Expected: all pass. CI additionally runs PostgreSQL/MySQL retention and migration contracts.

- [ ] **Step 7: Update state and report**

Mark only `Feed 保留策略` complete. Record exact tests, CI run, schema/index, configuration, concurrency behavior, and remaining RSS items in `.superpowers/sdd/feed-retention-v1-report.md`.

- [ ] **Step 8: Commit and push**

```bash
git add src/feeds/runtime.rs src/main.rs tests/feed_runtime.rs .env.example \
  docs/configuration.md tasks/todo.md tasks/plan.md \
  docs/superpowers/plans/2026-07-18-feed-retention-v1.md
git add -f .superpowers/sdd/feed-retention-v1-report.md
git commit -m "feat: run feed retention maintenance"
git push origin feature/foundation-bootstrap
```

## Plan self-review

- Spec coverage: configuration, index, bounded selector, locked recheck, cascade behavior, outbox survival, multi-instance idempotency, subscription race recovery, runtime scheduling, docs, and three-database tests each map to a task.
- DDIA consistency: the database is authoritative; no process-local lock participates in correctness; destructive decisions are made in short transactions after a lock; batch size and index bound load.
- API consistency: no new HTTP surface is added; configuration is additive; the subscription retry remains internal and does not expand public error contracts.
- Type consistency: `FeedRetentionConfig` is the configuration DTO, `FeedRetentionPolicy` is the runtime policy, and `FeedRetentionError` is the repository failure surface.
- Placeholder scan: no deferred implementation, undefined verification, or generic error-handling instruction remains.
- Scope exclusions: subscribed-entry pruning, refresh history pruning, outbox retention, UI, OPML, AI/plugin/MCP, and backup compaction remain separate slices.
