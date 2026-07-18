# Content Jobs / Artifacts Core v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `executing-plans` to implement this plan task-by-task. This plan is **inline main Agent only**: do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the durable, tenant-scoped content job/attempt/artifact core that every future Wasm AI plugin, Reader action, lifecycle trigger, API, and MCP entry point must reuse.

**Architecture:** Add four relational tables and focused SeaORM entities, then expose a validated `content::jobs` domain with canonical framed hashes and one repository contract across SQLite, PostgreSQL, and MySQL. The database is the job/artifact/audit record system; claims use database time, short leases, monotonic fencing tokens, immutable attempts, and atomic artifact/result/job terminal transactions.

**Tech Stack:** Rust 2024, SeaORM 1.1, SeaORM Migration, SQLite/PostgreSQL/MySQL, BLAKE3, serde/serde_json, time, Tokio tests.

## Global Constraints

- Work only in `/home/czyt/code/rust/raindrop/.worktrees/foundation-bootstrap` on `feature/foundation-bootstrap`.
- Main Agent executes inline; do not spawn or delegate to subagents.
- Do not modify or stage `.superpowers/research/` or root `node_modules/`.
- Do not use `git add -A`; stage only paths named by the current task.
- Use `apply_patch` for source/document edits.
- Run each task's focused verification before commit; push immediately after every commit.
- Only fix concrete verification or CI failures; do not start unbounded review loops.
- Rust edition remains `2024`; do not add a dependency unless the standard library and existing crates cannot satisfy the contract.
- SQLite, PostgreSQL, and MySQL share one public repository contract and semantic test suite.
- Database time decides due/lease/deadline ownership; process wall clock never grants commit authority.
- Maximum attempts is 3; initial lease is 30 seconds; manual/Reader/MCP timeout is 180 seconds; automatic timeout is 120 seconds.
- Per-user database-enforced active concurrency is 2; instance worker pool limit remains a later runtime composition concern capped at 8.
- Artifact candidate payload is at most 512 KiB canonical JSON; metadata/provenance are each at most 32 KiB canonical redacted JSON.
- No native processor, HTTP route, Reader component, lifecycle dispatcher, Wasm runtime, provider execution, or MCP call is added in this plan.
- Content artifacts are immutable derived data and never overwrite `entries.title`, `entries.summary`, or `entries.sanitized_content`.

---

## File Map

Create:

- `src/db/migration/content_jobs.rs` — four-table schema, indexes, FKs, reverse-order rollback.
- `src/db/entities/content_job.rs` — `content_jobs` SeaORM entity.
- `src/db/entities/content_job_attempt.rs` — `content_job_attempts` entity.
- `src/db/entities/content_artifact.rs` — immutable `content_artifacts` entity.
- `src/db/entities/content_job_result.rs` — job-to-artifact result entity.
- `src/content/jobs/mod.rs` — public exports.
- `src/content/jobs/model.rs` — validated enums, request/result/claim/error contracts.
- `src/content/jobs/hash.rs` — domain-separated framed BLAKE3 and canonical JSON.
- `src/content/jobs/repository.rs` — transaction orchestration and public methods.
- `src/content/jobs/sql.rs` — backend SQL, database clock, locks, conditional updates, row decoding.
- `tests/content_job_migration.rs` — migration/entity contract.
- `tests/content_job_primitives.rs` — validation and hash contract.
- `tests/content_job_enqueue.rs` — enqueue/idempotency/visibility/reuse contract.
- `tests/content_job_claims.rs` — claim/concurrency/heartbeat/recovery contract.
- `tests/content_job_terminals.rs` — failure/success/artifact atomicity contract.
- `tests/content_job_backend_contracts.rs` — shared SQLite/PostgreSQL/MySQL semantic suite.

Modify:

- `src/db/migration.rs` — register the content migration after `ai_providers`.
- `src/db/entities.rs` — export the four entities.
- `src/content/mod.rs` — export `jobs`.
- `tests/support/database.rs` — add focused provider/job fixture helpers without changing existing RSS fixtures.
- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md` — point the logical job model to the binding core specification.
- `docs/ai-providers.md` — document that provider calls must be composed through content claims in the next execution slice.
- `.superpowers/sdd/content-jobs-artifacts-core-v1-report.md` — evidence, commands, known exclusions, CI URL/status.

## Public Interfaces

The plan must converge on these names; implementation may add private helpers but must not rename the public boundary without updating every later task and this plan.

```rust
pub struct ContentRepository { /* DatabaseConnection */ }

impl ContentRepository {
    pub fn new(database: DatabaseConnection) -> Self;
    pub async fn enqueue(&self, request: EnqueueContentJob)
        -> Result<EnqueueResult, ContentRepositoryError>;
    pub async fn claim_next(&self, request: ClaimContentJob)
        -> Result<ClaimOutcome, ContentRepositoryError>;
    pub async fn heartbeat(&self, claim: &ContentJobClaim)
        -> Result<LeaseDeadline, ContentRepositoryError>;
    pub async fn complete_failure(
        &self,
        claim: &ContentJobClaim,
        failure: AttemptFailure,
    ) -> Result<JobSnapshot, ContentRepositoryError>;
    pub async fn complete_success(
        &self,
        claim: &ContentJobClaim,
        candidate: ArtifactCandidate,
        usage: AttemptUsage,
    ) -> Result<StoredArtifactResult, ContentRepositoryError>;
    pub async fn get_job(&self, user_id: &str, job_id: &str)
        -> Result<JobSnapshot, ContentRepositoryError>;
    pub async fn list_attempts(&self, user_id: &str, job_id: &str)
        -> Result<Vec<AttemptSnapshot>, ContentRepositoryError>;
    pub async fn get_result(&self, user_id: &str, job_id: &str)
        -> Result<StoredArtifactResult, ContentRepositoryError>;
    pub async fn find_artifact_by_identity(
        &self,
        user_id: &str,
        identity: &ArtifactIdentity,
    ) -> Result<Option<ArtifactSnapshot>, ContentRepositoryError>;
}
```

```rust
pub enum EnqueueResult {
    Queued(JobSnapshot),
    Reused { job: JobSnapshot, artifact: Box<ArtifactSnapshot> },
    Existing(JobSnapshot),
}

pub enum ClaimOutcome {
    Claimed(ContentJobClaim),
    RecoveredTerminal(JobSnapshot),
    NoWork,
}
```

---

### Task 1: Add the four-table migration and SeaORM entities

**Files:**

- Create: `src/db/migration/content_jobs.rs`
- Create: `src/db/entities/content_job.rs`
- Create: `src/db/entities/content_job_attempt.rs`
- Create: `src/db/entities/content_artifact.rs`
- Create: `src/db/entities/content_job_result.rs`
- Create: `tests/content_job_migration.rs`
- Modify: `src/db/migration.rs`
- Modify: `src/db/entities.rs`

**Interfaces:**

- Consumes: existing `operational_timestamp`, `users`, `entries`, `ai_providers` migration ordering.
- Produces: table/entity column names exactly matching section 6 of the design spec.

- [ ] **Step 1: Write the failing migration contract**

Add a SQLite test that runs `migrate`, queries `sqlite_master`/`PRAGMA foreign_key_list`/`PRAGMA index_list`, and asserts all four tables plus these exact indexes exist:

```rust
const EXPECTED_INDEXES: &[&str] = &[
    "uq_content_jobs_idempotency",
    "idx_content_jobs_due",
    "idx_content_jobs_user_status",
    "idx_content_jobs_entry",
    "idx_content_jobs_identity",
    "uq_content_job_attempt_number",
    "idx_content_attempts_job",
    "uq_content_artifact_identity",
    "idx_content_artifacts_entry",
    "idx_content_artifacts_producer",
    "idx_content_job_results_artifact",
];
```

The test must insert one valid entity row per table, read it back through SeaORM, then call `rollback` and assert the four tables are absent.

- [ ] **Step 2: Run the focused test and confirm RED**

Run:

```bash
cargo test --locked --test content_job_migration
```

Expected: compilation fails because the migration/entity modules do not exist.

- [ ] **Step 3: Implement migration and entities**

Register one `CreateContentJobs` migration after `CreateAiProviders`. Its `up` order is jobs → attempts → artifacts → results; `down` is results → artifacts → attempts → jobs. Use `string_len`, `big_integer`, `integer`, `boolean`, `text`, and `operational_timestamp`; do not use backend JSON/enum types.

Use these primary/foreign key actions:

```text
content_jobs.user_id             -> users.id       ON DELETE CASCADE
content_jobs.entry_id            -> entries.id     ON DELETE CASCADE
content_job_attempts.job_id      -> content_jobs.id ON DELETE CASCADE
content_artifacts.user_id        -> users.id       ON DELETE CASCADE
content_artifacts.entry_id       -> entries.id     ON DELETE CASCADE
content_artifacts.producer_job_id-> content_jobs.id ON DELETE RESTRICT
content_job_results.job_id       -> content_jobs.id ON DELETE CASCADE
content_job_results.artifact_id  -> content_artifacts.id ON DELETE RESTRICT
```

Every entity uses `String` for bounded strings/JSON text, `i32` for integer counters, `i64` for bigint, `bool`, and `OffsetDateTime`/`Option<OffsetDateTime>`.

- [ ] **Step 4: Run focused verification**

```bash
cargo fmt --all --check
cargo test --locked --test content_job_migration
cargo test --locked --test ai_provider_storage --test feed_terminal_backend_contracts
```

Expected: all commands exit 0; PostgreSQL/MySQL environment-dependent cases may print their existing skip messages.

- [ ] **Step 5: Commit and push**

```bash
git add src/db/migration.rs src/db/migration/content_jobs.rs src/db/entities.rs \
  src/db/entities/content_job.rs src/db/entities/content_job_attempt.rs \
  src/db/entities/content_artifact.rs src/db/entities/content_job_result.rs \
  tests/content_job_migration.rs
git commit -m "feat: persist content job records"
git push origin feature/foundation-bootstrap
```

---

### Task 2: Add validated domain primitives and canonical hashing

**Files:**

- Create: `src/content/jobs/mod.rs`
- Create: `src/content/jobs/model.rs`
- Create: `src/content/jobs/hash.rs`
- Create: `tests/content_job_primitives.rs`
- Modify: `src/content/mod.rs`

**Interfaces:**

- Consumes: provider `ProviderKind`, `time::OffsetDateTime`, `serde_json::Value`.
- Produces: all request/claim/artifact/error types used by Tasks 3–5.

- [ ] **Step 1: Write validation and hashing tests**

Cover exact storage values and round trips for:

```rust
ContentOperation::{Summarize, Translate}
ArtifactKind::{AiSummary, AiTranslation}
ContentTrigger::{ManualApi, ReaderSidecar, FeedRefreshPersisted, McpServer}
JobStatus::{Queued, Running, RetryWait, Succeeded, Failed}
AttemptStatus::{Running, Succeeded, Failed, Abandoned}
```

Add tests proving:

- `hash_frames([b"ab", b"c"]) != hash_frames([b"a", b"bc"])`;
- different domain contexts differ;
- `{"b":2,"a":1}` and `{"a":1,"b":2}` canonicalize identically;
- `Key` and `key` idempotency values differ;
- invalid UUIDs, hashes, locales, control characters, oversized keys/JSON, attempts, timeouts, metrics, and negative/overflow conversions fail.

- [ ] **Step 2: Run the focused test and confirm RED**

```bash
cargo test --locked --test content_job_primitives
```

Expected: compilation fails because `content::jobs` is absent.

- [ ] **Step 3: Implement small focused model/hash files**

Use typed constructors so repository methods cannot receive unchecked public fields. `ArtifactIdentity::hash()` must frame these exact values in this exact order:

```rust
[
    user_id,
    entry_id,
    artifact_kind.as_storage(),
    target_locale.unwrap_or(""),
    entry_content_hash,
    input_hash,
    config_hash,
    plugin_key,
    plugin_version,
    component_digest,
    provider_binding_id,
    provider_kind.as_storage(),
    provider_model,
    provider_revision_be_bytes,
    prompt_version,
    schema_id,
    mcp_provenance_hash,
]
```

Canonical JSON must parse to `serde_json::Value`, recursively use deterministic object key order, compact serialize, and reject outputs over the caller's byte ceiling. Do not log the input on error.

Define `ContentRepositoryErrorKind` with the exact variants from design section 11, and a non-sensitive `ContentRepositoryError` that exposes only `kind()`.

- [ ] **Step 4: Run focused verification**

```bash
cargo fmt --all --check
cargo test --locked --test content_job_primitives
cargo clippy --locked --lib --all-features -- -D warnings
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit and push**

```bash
git add src/content/mod.rs src/content/jobs/mod.rs src/content/jobs/model.rs \
  src/content/jobs/hash.rs tests/content_job_primitives.rs
git commit -m "feat: define content job contracts"
git push origin feature/foundation-bootstrap
```

---

### Task 3: Implement tenant-safe idempotent enqueue and artifact reuse

**Files:**

- Create: `src/content/jobs/repository.rs`
- Create: `src/content/jobs/sql.rs`
- Create: `tests/content_job_enqueue.rs`
- Modify: `src/content/jobs/mod.rs`
- Modify: `tests/support/database.rs`

**Interfaces:**

- Consumes: Task 1 entities; Task 2 `EnqueueContentJob`, `ArtifactIdentity`, `EnqueueResult`.
- Produces: `ContentRepository::new`, `enqueue`, `get_job`, `find_artifact_by_identity`.

- [ ] **Step 1: Write the enqueue contract tests**

Test visible entry queueing, disabled user/not-visible/missing uniform NotFound, entry hash drift, exact duplicate, semantic idempotency conflict, case-sensitive keys, concurrent same-key enqueue, and pre-existing artifact reuse.

The concurrency assertion is:

```rust
let ids = futures_util::future::join_all(requests)
    .await
    .into_iter()
    .map(expect_existing_or_queued_id)
    .collect::<HashSet<_>>();
assert_eq!(ids.len(), 1);
assert_eq!(content_job::Entity::find().count(&database).await.unwrap(), 1);
```

Add a test-only identity-hash override seam under `#[cfg(test)]` or construct stored collision rows directly; a hash collision must return `HashCollision`, never Existing/Reused.

- [ ] **Step 2: Run the focused test and confirm RED**

```bash
cargo test --locked --test content_job_enqueue
```

Expected: compile failure for missing repository methods.

- [ ] **Step 3: Implement lock helpers and enqueue transaction**

In `sql.rs`, implement:

```rust
pub(super) async fn database_now<C: ConnectionTrait>(...) -> Result<OffsetDateTime, ...>;
pub(super) async fn lock_user<C: ConnectionTrait>(..., user_id: &str) -> Result<LockedUser, ...>;
pub(super) async fn lock_job<C: ConnectionTrait>(..., job_id: &str) -> Result<Option<LockedJob>, ...>;
```

SQLite `lock_user` executes `UPDATE users SET id = id WHERE id = ?` then selects; PostgreSQL/MySQL select `FOR UPDATE`. Enqueue must keep the transaction short and follow User → Job/Artifact ordering.

The visibility query joins entry to an existing subscription for the same user/feed. Do not return a distinct authorization error. Compare the stored raw idempotency key and every artifact identity column after hash lookup.

When reuse succeeds, insert a terminal job with `attempts = 0`, `started_at = NULL`, `completed_at = database_now`, then insert `content_job_results(was_reused = true)` in the same transaction.

- [ ] **Step 4: Run focused and regression verification**

```bash
cargo fmt --all --check
cargo test --locked --test content_job_enqueue
cargo test --locked --test ai_provider_storage --test feed_subscription_contracts
cargo clippy --locked --lib --tests --all-features -- -D warnings
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit and push**

```bash
git add src/content/jobs/mod.rs src/content/jobs/repository.rs src/content/jobs/sql.rs \
  tests/content_job_enqueue.rs tests/support/database.rs
git commit -m "feat: enqueue content jobs idempotently"
git push origin feature/foundation-bootstrap
```

---

### Task 4: Implement claim, heartbeat, per-user concurrency, and crash recovery

**Files:**

- Create: `tests/content_job_claims.rs`
- Modify: `src/content/jobs/model.rs`
- Modify: `src/content/jobs/repository.rs`
- Modify: `src/content/jobs/sql.rs`

**Interfaces:**

- Consumes: queued/retry/running jobs from Task 3.
- Produces: `claim_next`, `heartbeat`, `ContentJobClaim`, `ClaimOutcome`, `LeaseDeadline`.

- [ ] **Step 1: Write deterministic claim/recovery tests**

Use paused Tokio time only for client scheduling; write database timestamps explicitly when a test must force due/expired state. Cover due ordering, future retry exclusion, two-worker race, per-user limit 2 with another user still claimable, heartbeat ceiling, stale token/owner, expired lease/deadline, recovery to attempt 2, and terminal recovery after attempt 3.

Every successful claim must assert:

```rust
assert_eq!(claim.attempt, expected_attempt);
assert_eq!(claim.lease_token, expected_token);
assert!(claim.lease_until <= claim.attempt_deadline_at);
```

- [ ] **Step 2: Run the focused test and confirm RED**

```bash
cargo test --locked --test content_job_claims
```

Expected: compile failure for missing claim methods/types.

- [ ] **Step 3: Implement bounded candidate scan and User-first revalidation**

Query at most 16 ordered candidate IDs while filtering out users that already have two effective RUNNING rows, then for each candidate open a transaction, lock user, lock job, and revalidate due/status. The query-side count is only a liveness filter; the User lock and in-transaction count enforce correctness. Count only RUNNING rows whose lease and deadline are both after database now.

Recovery must first update the previous RUNNING attempt to:

```text
status=ABANDONED
error_code=JOB_LEASE_EXPIRED
retryable=true
outcome_unknown=false
completed_at=database_now
```

If attempts remain, allocate one new UUID attempt, increment token and attempt number, and set `attempt_deadline_at = database_now + timeout_seconds`. If max attempts are exhausted, terminalize with `JOB_ATTEMPTS_EXHAUSTED` and return `RecoveredTerminal`.

Heartbeat is a conditional update that rejects dead/expired claims and clamps extension to the fixed attempt deadline.

- [ ] **Step 4: Run focused and contention verification**

```bash
cargo fmt --all --check
cargo test --locked --test content_job_claims -- --test-threads=1
cargo test --locked --test feed_refresh_claims
cargo clippy --locked --lib --tests --all-features -- -D warnings
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit and push**

```bash
git add src/content/jobs/model.rs src/content/jobs/repository.rs src/content/jobs/sql.rs \
  tests/content_job_claims.rs
git commit -m "feat: recover leased content jobs"
git push origin feature/foundation-bootstrap
```

---

### Task 5: Implement failure scheduling and atomic artifact completion

**Files:**

- Create: `tests/content_job_terminals.rs`
- Create: `tests/content_job_backend_contracts.rs`
- Modify: `src/content/jobs/model.rs`
- Modify: `src/content/jobs/repository.rs`
- Modify: `src/content/jobs/sql.rs`

**Interfaces:**

- Consumes: valid `ContentJobClaim` from Task 4, canonical candidate/usage from Task 2.
- Produces: `complete_failure`, `complete_success`, `list_attempts`, `get_result`, artifact reads.

- [ ] **Step 1: Write terminal and backend contract tests**

Create one reusable function:

```rust
async fn backend_content_job_contract(url: &str) {
    enqueue_contract(url).await;
    claim_and_recovery_contract(url).await;
    failure_contract(url).await;
    artifact_contract(url).await;
    tenant_contract(url).await;
}
```

Call it from SQLite unconditionally and PostgreSQL/MySQL when their existing environment variables are present. Cover permanent failure, 5s/30s retry, Retry-After clamp, unknown outcome, stale completion rejection, new artifact success, artifact reuse across different jobs, immutable entry content, tenant reads, and identity provenance changes.

Add SQLite failure triggers before result insert and before job terminal update; both must leave artifact/result absent and attempt/job RUNNING.

- [ ] **Step 2: Run the focused tests and confirm RED**

```bash
cargo test --locked --test content_job_terminals --test content_job_backend_contracts
```

Expected: compile failure for missing terminal/read methods.

- [ ] **Step 3: Implement failure transaction and retry calculation**

Use the exact schedule:

```rust
fn retry_delay(attempt: u8, retry_after: Option<Duration>) -> Duration {
    let base = match attempt {
        1 => Duration::from_secs(5),
        2 => Duration::from_secs(30),
        _ => Duration::ZERO,
    };
    base.max(retry_after.unwrap_or_default())
        .min(Duration::from_secs(60 * 60))
}
```

Validate and persist metrics only after claim ownership is proven. On RETRY_WAIT clear owner/lease/deadline but keep `completed_at = NULL`; on FAILED set terminal time.

- [ ] **Step 4: Implement atomic success and reads**

Before opening the transaction, canonicalize/check payload and provenance. Inside the User → Job → Attempt → Artifact → Result transaction, revalidate claim, compare complete identity after hash lookup, insert or reuse artifact, insert result, complete attempt, then complete job. Any affected-row count other than one is `LeaseLost`/`CorruptData` and rolls back.

`list_attempts` orders by attempt ascending. `get_result` joins result/artifact through a user-scoped job. Cross-user IDs and missing IDs both return NotFound.

- [ ] **Step 5: Run focused and backend verification**

```bash
cargo fmt --all --check
cargo test --locked --test content_job_terminals --test content_job_backend_contracts -- --test-threads=1
cargo test --locked --test ai_provider_storage --test feed_terminal_backend_contracts \
  --test feed_lifecycle_outbox
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Expected: all commands exit 0; unconfigured external backend cases print skip messages, not failures.

- [ ] **Step 6: Commit and push**

```bash
git add src/content/jobs/model.rs src/content/jobs/repository.rs src/content/jobs/sql.rs \
  tests/content_job_terminals.rs tests/content_job_backend_contracts.rs
git commit -m "feat: commit immutable content artifacts"
git push origin feature/foundation-bootstrap
```

---

### Task 6: Bind documentation, run full verification, and record CI

**Files:**

- Create: `.superpowers/sdd/content-jobs-artifacts-core-v1-report.md`
- Modify: `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- Modify: `docs/ai-providers.md`

**Interfaces:**

- Consumes: all Task 1–5 evidence.
- Produces: implementation report and a clean pushed branch with CI evidence.

- [ ] **Step 1: Update architecture documentation**

In the AI plugin spec, retain its logical tables but add a binding note to `2026-07-19-content-jobs-artifacts-core-v1-design.md` for physical schema, leases, result links, hashing, and transactions. In provider docs, state that `ProviderClient` remains transport/core and future plugin execution must be invoked only while holding a valid content job claim.

- [ ] **Step 2: Run fresh full verification**

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

Expected: all exit 0. Record exact passed/failed/ignored counts; the existing live IT之家 RSS smoke may remain ignored unless `RAINDROP_LIVE_RSS_SMOKE=1` is set.

- [ ] **Step 3: Write the implementation report**

The report must include:

```text
Scope delivered
Schema and state-machine decisions
SQLite/PostgreSQL/MySQL evidence
Focused and full command results with counts
Security/tenant/secret review
Known exclusions: Wasm, provider execution composition, MCP, lifecycle, API, Reader/UI
Commit hashes and CI run URL/status
```

Do not mark future AI plugin todo items complete.

- [ ] **Step 4: Commit documentation and push**

```bash
git add docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md docs/ai-providers.md \
  .superpowers/sdd/content-jobs-artifacts-core-v1-report.md
git commit -m "docs: record content job core"
git push origin feature/foundation-bootstrap
```

- [ ] **Step 5: Watch the pushed CI once and fix only concrete failures**

Use the repository's existing GitHub Actions status command. If all jobs pass, append the run URL/status to the report in a `[skip ci]` commit and push. If a job fails, reproduce the exact failure locally when possible, apply one bounded fix, rerun its proof command plus the full relevant gate, commit, and push. Do not open a speculative re-review cycle after green CI.

---

## Plan Self-Review

- Spec coverage: migration/entities, hash/model validation, tenant-safe enqueue, artifact reuse, claim/lease/fencing, per-user concurrency, recovery, retry/unknown outcome, atomic terminal transaction, reads, three-backend suite, docs and full verification all map to Tasks 1–6.
- Scope boundary: no plugin runtime, provider execution, MCP, lifecycle dispatcher, API, Reader or UI code is introduced.
- Type consistency: the public repository/type names in Tasks 3–5 match the Public Interfaces section.
- Placeholder scan: no unresolved marker, deferred implementation instruction, or unspecified test step remains.
- Execution choice: fixed by user instruction to inline main Agent execution with `executing-plans`; no user confirmation checkpoint is required.
