# Raindrop Subscription API v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver queue-only Subscription CRUD/refresh APIs plus a durable background Feed runtime so a user can subscribe, observe, read, refresh, and unsubscribe without HTTP waiting on network I/O.

**Architecture:** Keep database command/query semantics in `feeds`, split queue-only `FeedCommandService` from claim-only `FeedExecutor`, and make `FeedRuntime` the sole owner of stale-run recovery, scheduled enqueue, claiming, heartbeat, and shutdown. Expose user-scoped wire DTOs through one scoped Axum router; database records remain authoritative when Notify, clients, or workers disappear.

**Tech Stack:** Rust 2024, MSRV 1.94, Axum 0.8.9, Tokio 1.52, SeaORM 1.1.19, existing BLAKE3/base64/URL/fetch/parser/sanitizer stack, SQLite/PostgreSQL/MySQL.

## Global Constraints

- Cargo verification commands use `--locked`.
- No new crate and no database migration.
- HTTP mutation never waits for DNS, network, parser, sanitizer, AI, MCP, or plugin work.
- All user data reads/writes authorize through `CurrentUser` and user-owned Subscription; mutation also requires `CsrfGuard`.
- Every enqueue/recovery path uses the shared Feed-first locked transaction contract; user commands lock user before Feed, runtime never locks user.
- Initial visible history is exactly the most recent 100 persisted entries at the locked Feed head.
- Per-user ceilings are exactly 1,000 subscriptions, 20 user-requested active refresh runs, and 30 HTTP mutations per 15 minutes.
- Runtime uses exactly 2 lanes, a 60-second lease, a 20-second heartbeat, a 30-second manual cooldown, a 1-second idle poll, and a 30-second scheduled scan.
- All Subscription API success/error/fallback responses use `Cache-Control: no-store` and `Pragma: no-cache`.
- Domain/database DTOs do not gain HTTP Serde derives; wire DTOs map explicitly.
- Only Critical/Important findings block; use one bounded fix wave and one re-review per task.
- Live IT Home smoke is non-blocking release evidence; deterministic local and three-backend contracts are blocking.
- Never modify the user's untracked `.superpowers/research/` or `node_modules/`.

---

### Task 1: User-scoped Subscription list/detail projection

**Files:**
- Modify: `src/feeds/dto.rs`
- Modify: `src/feeds/subscription.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_subscription_contracts.rs`

**Interfaces:**
- Consumes: existing `FeedRepository`, `subscriptions`, `feeds`, `entries`, `entry_states`, `feed_refresh_runs`.
- Produces: `ListSubscriptionsQuery`, `SubscriptionListItemDto`, `SubscriptionPage`, `FeedRepository::list_subscriptions_for_user`, `FeedRepository::get_subscription_for_user`.

- [ ] **Step 1: Write the SQLite projection tracer**

Create `tests/feed_subscription_contracts.rs` with a migrated SQLite fixture containing two users, shared Feed, entries, read overrides, title override, and refresh runs. First test:

```rust
#[tokio::test]
async fn sqlite_subscription_list_is_user_scoped_and_matches_reader_unread_state() {
    let fixture = SubscriptionFixture::new().await;
    let page = fixture
        .repository
        .list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery { cursor: None, limit: 1 },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(page.items[0].title, "Personal title");
    assert_eq!(page.items[0].unread_count, 2);
    assert!(page.next_cursor.is_some());
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --locked --all-features --test feed_subscription_contracts sqlite_subscription_list
```

Expected: compile failure because the DTO/query methods do not exist.

- [ ] **Step 3: Add exact domain DTOs**

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListSubscriptionsQuery {
    pub cursor: Option<String>,
    pub limit: u16,
}

impl Default for ListSubscriptionsQuery {
    fn default() -> Self {
        Self { cursor: None, limit: 50 }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionListItemDto {
    pub subscription_id: String,
    pub feed_id: String,
    pub title: String,
    pub site_url: Option<String>,
    pub unread_count: i64,
    pub refresh: Option<RefreshDto>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionPage {
    pub items: Vec<SubscriptionListItemDto>,
    pub next_cursor: Option<String>,
}
```

Give `SubscriptionListItemDto` a redacted `Debug` matching existing feed/entry DTO policy.

- [ ] **Step 4: Implement canonical cursor and projection query**

Cursor v1 canonical payload:

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubscriptionCursorV1 {
    version: u8,
    user_hash: String,
    order: String,
    created_at_us: i64,
    subscription_id: String,
}
```

Use URL-safe base64 without padding and reject decode/re-encode differences. `user_hash` is the existing stable framed BLAKE3/base64url pattern over user ID; `order` is exactly `CREATED_DESC_ID_DESC`.

Implement one SQL projection per backend using a CTE/window for the latest run and a correlated unread count. Order by `s.created_at DESC, s.id DESC`, fetch `limit + 1`, and never select source/fetch URL or validators into DTOs.

- [ ] **Step 5: Add detail and validation cycles**

Add tests one at a time:

```text
sqlite_subscription_detail_hides_missing_and_cross_tenant
sqlite_subscription_list_rejects_invalid_user_limit_and_cursor
sqlite_subscription_cursor_rejects_cross_user_and_noncanonical_reuse
sqlite_subscription_title_falls_back_to_feed_then_host
sqlite_subscription_latest_refresh_uses_queued_at_then_run_id
```

Run each exact filter RED then GREEN.

- [ ] **Step 6: Add PostgreSQL/MySQL explain/runtime filters**

Add env-gated backend cases to the same test file. They must use existing backend serial CI service conventions and assert the user-leading subscription index plus bounded entry/feed access; local missing URLs explicitly skip.

- [ ] **Step 7: Verify and commit**

```bash
cargo test --locked --all-features --test feed_subscription_contracts
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/feeds/dto.rs src/feeds/subscription.rs src/feeds/mod.rs tests/feed_subscription_contracts.rs
git commit -m "feat: query user subscriptions"
```

---

### Task 2: Atomic Subscription commands and refresh admission

**Files:**
- Modify: `src/feeds/dto.rs`
- Modify: `src/feeds/refresh.rs`
- Modify: `src/feeds/repository.rs`
- Modify: `src/feeds/subscription.rs`
- Modify: `tests/feed_subscription_contracts.rs`

**Interfaces:**
- Consumes: Task 1 projection methods and current refresh run schema.
- Produces: `SubscribeOutcome`, `QueueSubscriptionRefresh`, atomic create/reuse/manual/unsubscribe repository contracts, typed capacity/cooldown/in-progress results.

- [ ] **Step 1: Write the initial-history tracer**

```rust
#[tokio::test]
async fn sqlite_new_subscription_sees_at_most_one_hundred_existing_entries_as_unread() {
    let fixture = SubscriptionFixture::with_feed_head(150).await;
    let outcome = fixture.subscribe_user_b().await.unwrap();
    assert!(outcome.created);
    assert_eq!(outcome.subscription.unread_count, 100);
    let row = fixture.subscription_row(USER_B_ID).await;
    assert_eq!(row.start_sequence, 50);
    assert_eq!(row.read_through_sequence, 50);
}
```

Run RED with the exact name.

- [ ] **Step 2: Add command types and errors**

```rust
pub struct SubscribeOutcome {
    pub created: bool,
    pub subscription: SubscriptionListItemDto,
}

pub struct QueueSubscriptionRefresh {
    pub request_id: String,
}

```

`queue_subscription_refresh` returns `RefreshDto` directly: newly accepted work is QUEUED, exact idempotent replay may be any persisted status, and another active request is a typed error rather than a second success variant.

Extend `RefreshDto` with the already-persisted fields:

```rust
pub error_code: Option<String>,
pub retry_at: Option<OffsetDateTime>,
pub queued_at: OffsetDateTime,
pub started_at: Option<OffsetDateTime>,
pub completed_at: Option<OffsetDateTime>,
```

Update every existing constructor/query in the same task; do not fill these fields with application-clock placeholders.

Add typed internal errors for `SubscriptionLimit`, `ActiveRefreshLimit`, `RefreshInProgress { operation_id }`, `RefreshCooldown { retry_at }`, `FeedDisabled`, and idempotency conflict. All `Debug` output redacts IDs and timestamps where disclosure is unnecessary.

- [ ] **Step 3: Implement shared Feed-first lock helper**

Add an internal transaction helper whose contract is:

```text
lock active user (user commands only)
→ lock Feed
→ read exact key
→ read active QUEUED/RUNNING
→ revalidate due/disabled/orphan/capacity
→ insert/return/reject
```

PostgreSQL/MySQL use `FOR UPDATE`. SQLite's first Feed statement is:

```sql
UPDATE feeds
SET lease_token = lease_token
WHERE id = ?
```

and must affect exactly one row.

- [ ] **Step 4: Implement create/reuse semantics**

Change the Subscription insert frontier to:

```rust
const INITIAL_VISIBLE_ENTRY_COUNT: i64 = 100;
let initial_frontier = entry_sequence_head.saturating_sub(INITIAL_VISIBLE_ENTRY_COUNT);
```

Within the user→Feed transaction:

- enforce 1,000 subscriptions/user and 20 user-requested active runs;
- duplicate existing Subscription returns `created=false` and never queues;
- new Subscription queues `subscribe:{subscription_id}` only if Feed is new, never succeeded, or due;
- fresh shared Feed returns history without a network run;
- an already active Feed run is referenced, not duplicated.

- [ ] **Step 5: Implement exact manual idempotency**

Use:

```rust
fn manual_idempotency_key(user_id: &str, request_id: &str) -> String {
    let digest = stable_framed_blake3(&[b"manual-refresh-v1", user_id.as_bytes(), request_id.as_bytes()]);
    format!("m1:{}", URL_SAFE_NO_PAD.encode(digest.as_bytes()))
}
```

Assert length is exactly 46. Under Feed lock: exact key first; other active returns typed in-progress without accepting request; cooldown is `max(last_attempt + 30s, retry_after_at)`; then capacity and insert.

- [ ] **Step 6: Implement idempotent unsubscribe**

Pre-read candidate Feed ID, then user→Feed lock, recheck `(subscription,user,feed)`, delete relationship, and set `orphaned_at=database now` only if no subscriptions remain. Missing/cross-tenant returns `false`. Re-subscribe clears orphan marker.

- [ ] **Step 7: Add concurrency and boundary tests**

Add exact tests:

```text
sqlite_same_user_concurrent_subscribe_creates_one_relationship
sqlite_two_users_share_feed_and_fresh_history_without_duplicate_run
sqlite_existing_head_sixty_exposes_sixty_unread_entries
sqlite_due_feed_concurrent_subscribe_creates_one_active_run
sqlite_manual_exact_request_replays_terminal_run_before_cooldown
sqlite_manual_different_request_rejects_while_active
sqlite_manual_key_fits_all_backend_limits
sqlite_manual_cooldown_respects_retry_after
sqlite_subscription_and_active_run_quotas_are_atomic
sqlite_unsubscribe_is_idempotent_and_marks_only_last_feed_orphan
sqlite_concurrent_resubscribe_clears_orphan
```

Add PostgreSQL/MySQL versions for queue race, quota, and orphan/resubscribe using existing serial services.

- [ ] **Step 8: Verify and commit**

```bash
cargo test --locked --all-features --test feed_subscription_contracts
cargo test --locked --all-features --test feed_refresh_claims
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/feeds/dto.rs src/feeds/refresh.rs src/feeds/repository.rs src/feeds/subscription.rs tests/feed_subscription_contracts.rs
git commit -m "feat: persist subscription commands"
```

---

### Task 3: Split queue-only commands from claim-only execution

**Files:**
- Modify: `src/feeds/service.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `tests/feed_ingestion_e2e.rs`
- Modify: `tests/live_rss_ithome.rs`
- Create: `tests/feed_executor_contracts.rs`

**Interfaces:**
- Consumes: Task 2 repository commands, existing fetch/parse/persist/schedule logic.
- Produces: `FeedCommandService`, `FeedExecutor<T>`, `FeedExecutor::execute_claim`.

- [ ] **Step 1: Write queue-only command tracer**

Use a fake transport blocked behind a `Notify` and assert `FeedCommandService::subscribe` returns before the gate is released and execution count remains zero.

- [ ] **Step 2: Run RED**

```bash
cargo test --locked --all-features --test feed_executor_contracts command_service_never_calls_transport
```

- [ ] **Step 3: Define exact service split**

```rust
#[derive(Clone)]
pub struct FeedCommandService {
    repository: FeedRepository,
    url_policy: FeedUrlPolicy,
}

pub struct FeedExecutor<T: FeedTransport> {
    repository: FeedRepository,
    url_policy: FeedUrlPolicy,
    transport: T,
    parser: FeedParser,
    schedule: Mutex<RefreshSchedule<Box<dyn JitterSource + Send>>>,
}
```

`FeedCommandService` implements list/detail/subscribe/queue refresh/unsubscribe and never owns transport. `FeedExecutor::execute_claim(claim)` starts from `load_refresh_context`, performs fetch/parse/persist/terminal completion, and never calls `claim_run/claim_due`.

- [ ] **Step 4: Move existing execution without duplication**

Move the body after claim acquisition from current `execute_run` into `execute_claim`. Delete `CLAIM_ATTEMPTS`, `CLAIM_RETRY_DELAY`, `execute_run`, and synchronous network behavior from command methods. Retain exact error classification and schedule logic.

- [ ] **Step 5: Add executor contracts**

```text
executor_success_persists_and_returns_refresh
executor_not_modified_never_parses_body
executor_fetch_parse_content_failures_end_with_stable_internal_codes
executor_rejects_claim_feed_mismatch_without_network
command_subscribe_and_manual_refresh_only_queue
```

Update existing IT Home scripted E2E and the opt-in live smoke to call command service, then claim and execute through the public runtime-facing seam. This is required before `clippy --all-targets` because the old synchronous `FeedService` entry points no longer exist.

- [ ] **Step 6: Verify and commit**

```bash
cargo test --locked --all-features --test feed_executor_contracts
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test live_rss_ithome --no-run
cargo test --locked --all-features --test feed_refresh_claims
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/feeds/service.rs src/feeds/mod.rs tests/feed_ingestion_e2e.rs tests/live_rss_ithome.rs tests/feed_executor_contracts.rs
git commit -m "refactor: split feed commands and execution"
```

---

### Task 4: Durable Feed runtime, recovery, scheduling, and heartbeat

**Files:**
- Create: `src/feeds/runtime.rs`
- Modify: `src/feeds/repository.rs`
- Modify: `src/feeds/mod.rs`
- Create: `tests/feed_runtime.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `FeedExecutor::execute_claim`, repository queue primitives, `SetupService`.
- Produces: `FeedRuntime`, `FeedRuntimeHandle`, stale recovery and scheduled enqueue methods.

- [ ] **Step 1: Write crash recovery RED tracer**

Seed a RUNNING run with expired lease, invoke one runtime maintenance cycle, and assert old run becomes LEASE_LOST and exactly one `r1:{old_run_id}` RETRY is queued.

```bash
cargo test --locked --all-features --test feed_runtime expired_running_run_recovers_to_one_retry
```

- [ ] **Step 2: Implement atomic stale recovery**

Add repository method:

```rust
pub async fn recover_expired_runs(
    &self,
    limit: u16,
) -> Result<Vec<String>, RefreshRepositoryError>;
```

Scan candidates, then per candidate Feed-first lock/recheck; terminalize old run and optionally insert `r1:{run_id}` in one transaction. If another active run exists, do not insert retry. Return newly queued run IDs for observability/tests.

- [ ] **Step 3: Implement scheduled enqueue**

```rust
pub async fn enqueue_due_scheduled(
    &self,
    limit: u16,
) -> Result<usize, RefreshRepositoryError>;
```

Use scan snapshot `(feed_id,next_fetch_at_us)`, then Feed-lock revalidation and `s1:` framed digest. Exclude disabled, orphan, no-subscription, and active-run feeds.

- [ ] **Step 4: Implement runtime handle and two lanes**

```rust
type ExecutorFactory<T> = Arc<
    dyn Fn(DatabaseConnection) -> Result<Arc<FeedExecutor<T>>, FeedServiceError> + Send + Sync,
>;

#[derive(Clone)]
pub struct FeedRuntimeHandle {
    notify: Arc<Notify>,
    shutdown_tx: watch::Sender<bool>,
}

pub struct FeedRuntime<T: FeedTransport> {
    setup: SetupService,
    executor_factory: ExecutorFactory<T>,
    notify: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
}
```

Do not add a crate for cancellation; use an existing Tokio `watch` channel. Start exactly two lane futures in a `JoinSet`. Setup-required state waits without constructing transport. Lane idle wait is Notify or one second; scheduler maintenance is one designated lane every 30 seconds.

- [ ] **Step 5: Add heartbeat structured concurrency**

For each claim, run attempt and 20-second lease extension under one `tokio::select!`. Attempt terminal completion first stops heartbeat before returning. Heartbeat failure cancels attempt; LeaseLost triggers recovery. Shutdown stops new claims and waits at most 30 seconds.

- [ ] **Step 6: Add deterministic runtime cases**

```text
setup_required_runtime_makes_zero_transport_calls
notify_and_poll_both_wake_queued_work
two_lanes_run_different_feeds_concurrently
two_lanes_never_run_same_feed_concurrently
heartbeat_extends_lease_before_deadline
terminal_completion_stops_heartbeat_without_false_lease_lost
heartbeat_lease_loss_cancels_attempt_and_queues_retry
expired_running_run_recovers_to_one_retry
multi_instance_recovery_and_scheduled_enqueue_are_idempotent
scheduled_enqueue_skips_disabled_orphan_unsubscribed_and_active
graceful_shutdown_stops_new_claims
```

Add PostgreSQL/MySQL serial recovery/scheduler filters to CI in this task if the existing backend job does not select them.

- [ ] **Step 7: Verify and commit**

```bash
cargo test --locked --all-features --test feed_runtime
cargo test --locked --all-features --test feed_refresh_claims
cargo test --locked --all-features --test feed_ingestion_e2e
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/feeds/runtime.rs src/feeds/repository.rs src/feeds/mod.rs tests/feed_runtime.rs .github/workflows/ci.yml
git commit -m "feat: run durable feed workers"
```

---

### Task 5: Application lifecycle and keyed mutation limiter

**Files:**
- Modify: `src/api/rate_limit.rs`
- Modify: `src/app.rs`
- Modify: `src/main.rs`
- Modify: `tests/feed_runtime.rs`

**Interfaces:**
- Consumes: Task 4 `FeedRuntime`/handle.
- Produces: `UserMutationLimiter`, AppState feed handle, main startup/shutdown integration.

- [ ] **Step 1: Add keyed limiter RED tests**

Unit tests in `src/api/rate_limit.rs`:

```text
user_mutation_limiter_isolates_users_at_exact_threshold
user_mutation_limiter_returns_ceil_retry_after_at_least_one
user_mutation_limiter_expires_and_bounds_key_storage
```

- [ ] **Step 2: Implement bounded keyed limiter**

```rust
#[derive(Clone)]
pub struct UserMutationLimiter {
    inner: Arc<Mutex<UserMutationLimiterState>>,
    limit: u32,
    window: Duration,
    max_keys: usize,
}

pub struct RateLimitRejection {
    pub retry_at: OffsetDateTime,
    pub retry_after_seconds: u64,
}
```

Use exact values 30/15 minutes/10,000. Expired entries are removed before capacity eviction; rejection never mutates another user's bucket.

- [ ] **Step 3: Wire AppState**

Add `feed_runtime: FeedRuntimeHandle` and `subscription_mutation_limiter: UserMutationLimiter`. Preserve `AppState::new(setup)` as a test/backward-compatible constructor with an inert handle, and add `AppState::with_feed_runtime(setup, handle)` for production. Neither constructor spawns network work by itself.

- [ ] **Step 4: Start runtime in main**

Construct one production handle/runtime, spawn runtime before `axum::serve`, and on shutdown signal request runtime shutdown before awaiting server termination. The runtime must survive setup-required mode and begin after SetupService exposes a database.

- [ ] **Step 5: Add lifecycle tests**

Prove setup transition wakes runtime, main-style shutdown stops lanes, and dropping/closing Notify does not change committed command outcome.

- [ ] **Step 6: Verify and commit**

```bash
cargo test --locked --all-features api::rate_limit
cargo test --locked --all-features --test feed_runtime
cargo test --locked --all-features --test version_fast_path
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/api/rate_limit.rs src/app.rs src/main.rs tests/feed_runtime.rs
git commit -m "feat: wire feed runtime lifecycle"
```

---

### Task 6: Subscription HTTP API seam

**Files:**
- Create: `src/api/subscriptions.rs`
- Modify: `src/api/error.rs`
- Modify: `src/api/mod.rs`
- Modify: `src/api/routes.rs`
- Create: `tests/subscription_api.rs`

**Interfaces:**
- Consumes: `FeedCommandService`, Task 5 limiter/runtime handle, CurrentUser, CsrfGuard, ApiJson.
- Produces: five `/api/v1/subscriptions` endpoints with stable wire DTOs.

- [ ] **Step 1: Add list/create tracer tests**

First tests through `build_router`:

```rust
#[tokio::test]
async fn subscription_list_is_empty_for_a_new_user() {
    let fixture = SubscriptionApiFixture::new().await;
    let response = fixture.get("/api/v1/subscriptions", UserKind::A).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    assert_eq!(response_json(response).await["items"], json!([]));
}

#[tokio::test]
async fn subscription_create_returns_before_blocked_transport_and_sets_location() {
    let fixture = SubscriptionApiFixture::with_blocked_transport().await;
    let response = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key(LOCATION));
    let body = response_json(response).await;
    assert_eq!(body["created"], true);
    assert_eq!(body["subscription"]["refresh"]["state"], "PENDING");
    assert_eq!(fixture.transport_calls(), 0);
}
```

Run RED:

```bash
cargo test --locked --all-features --test subscription_api subscription_
```

- [ ] **Step 2: Implement scoped router topology**

Use inner `/`, `/{subscription_id}`, `/{subscription_id}/refresh`, inner fallback/method fallback, outer explicit `/api/v1/subscriptions/`, and cache middleware around the entire scoped router. Never add a conflicting catch-all.

- [ ] **Step 3: Implement explicit wire DTOs**

Request DTOs are strict camelCase/deny unknown. Response DTOs expose exactly the spec fields. Map internal refresh status to `PENDING|READY|DEGRADED|BACKING_OFF|ERROR`; time formatting is fixed UTC RFC3339 with six microseconds and `Z`; internal error code maps only to `REFRESH_FAILED|UPSTREAM_RATE_LIMITED`.

- [ ] **Step 4: Implement handlers and precedence**

Handler signatures:

```rust
async fn create_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateSubscriptionRequest>,
) -> Result<Response, ApiError>;

async fn refresh_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(subscription_id): Path<String>,
    ApiJson(request): ApiJson<RefreshSubscriptionRequest>,
) -> Result<Response, ApiError>;
```

Call the keyed limiter only after extractor and request UUID/URL validation. Database exact replay remains prior to database cooldown. Notify after successful commit, ignoring a closed Notify channel.

- [ ] **Step 5: Extend ApiError for 429 headers without leaking internals**

Add optional response headers and reuse `fields.retryAt`; `IntoResponse` attaches `Retry-After`. `REFRESH_IN_PROGRESS` stores only public `operationId` in fields. Conflict/hash/disabled/idempotency all use stable 409 envelopes.

- [ ] **Step 6: Add security/error classes one RED→GREEN cycle each**

```text
subscription_routes_require_active_session
subscription_mutations_require_valid_csrf
subscription_mutations_enforce_same_origin
subscription_requests_reject_invalid_query_path_body_and_url
subscription_detail_hides_missing_and_cross_tenant
subscription_delete_is_idempotent_and_non_enumerating
subscription_refresh_is_exactly_idempotent_and_reports_active_conflict
subscription_rate_limits_are_user_scoped_and_return_retry_after
subscription_responses_disable_caching_for_200_201_202_204_401_403_404_405_409_422_429_500
subscription_unknown_and_trailing_paths_never_return_embedded_html
```

- [ ] **Step 7: Run focused regressions and commit**

```bash
cargo test --locked --all-features --test subscription_api
cargo test --locked --all-features --test feed_subscription_contracts
cargo test --locked --all-features --test feed_runtime
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features --test session_security
cargo test --locked --all-features --test setup_auth_api
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
git add src/api/subscriptions.rs src/api/error.rs src/api/mod.rs src/api/routes.rs tests/subscription_api.rs
git commit -m "feat: expose subscription api"
```

---

### Task 7: OpenAPI drift gate and whole-slice verification

**Files:**
- Create: `docs/openapi/subscription-v1.json`
- Create: `tests/openapi_contract.rs`
- Modify: `docs/superpowers/specs/2026-07-16-raindrop-design.md`
- Modify: `tasks/todo.md`

**Interfaces:**
- Consumes: Task 6 real router and wire contract.
- Produces: committed API artifact used by the next ASTRYX client slice.

- [ ] **Step 1: Write failing drift test**

Test loads `docs/openapi/subscription-v1.json`, drives every documented success/error through `build_router`, and checks route, status, required fields, enum values, `Location`, `Retry-After`, content type, and cache headers. It also fails if artifact declares an unimplemented endpoint.

- [ ] **Step 2: Add exact OpenAPI fragment**

Document the five endpoints, request/response/error schemas, public refresh enum, 201 Location, 429 Retry-After, 204 empty body, and shared error envelope. Do not include internal Feed URL, run status, lease, frontier, revision, SQL, or provider error.

- [ ] **Step 3: Update roadmap docs**

Mark Subscription API/runtime complete in `tasks/todo.md`; update the overall design endpoint from `GET /feeds` to user-scoped `GET /subscriptions` and add detail route. Record that TypeScript generation in the first Reader client task must consume this artifact.

- [ ] **Step 4: Run fresh completion gates**

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features --test openapi_contract
cargo test --locked --all-features --test subscription_api
cargo test --locked --all-features --test feed_subscription_contracts
cargo test --locked --all-features --test feed_executor_contracts
cargo test --locked --all-features --test feed_runtime
cargo test --locked --all-features --test feed_refresh_claims
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features
git diff --check
git status --short
```

Expected: all blocking commands pass. Status contains only intended tracked changes and the user's pre-existing untracked `.superpowers/research/` and `node_modules/`.

- [ ] **Step 5: Run optional live release smoke**

```bash
RAINDROP_LIVE_RSS_SMOKE=1 cargo test --locked --all-features --test live_rss_ithome -- --ignored --nocapture
```

Record 50..=100 entries and either NOT_MODIFIED/304 or deduplicated SUCCESS/200. Failure is reported as external non-blocking evidence, not used to falsify deterministic gates.

- [ ] **Step 6: Commit**

```bash
git add docs/openapi/subscription-v1.json tests/openapi_contract.rs docs/superpowers/specs/2026-07-16-raindrop-design.md docs/superpowers/specs/2026-07-17-subscription-api-v1-design.md docs/superpowers/plans/2026-07-17-subscription-api-v1.md tasks/todo.md
git commit -m "docs: freeze subscription api contract"
```

## Plan self-review

- Spec coverage: list/detail/create/manual refresh/unsubscribe, initial 100-entry window, quotas, exact idempotency, atomic queue, stale recovery, scheduled enqueue, executor split, runtime lifecycle, HTTP security/cache, OpenAPI, three backends, and optional live smoke each map to a Task.
- Dependency order: projection → commands → executor → runtime → app lifecycle → HTTP → OpenAPI/final gate.
- Type consistency: Task 1 produces page/item DTOs; Task 2 adds outcomes/admission; Task 3 produces command/executor; Task 4 produces runtime handle; Task 5 installs it; Task 6 consumes all; Task 7 freezes the public artifact.
- Placeholder scan: no TBD/TODO/“similar to” implementation marker remains.
- Review policy: fresh implementer per Task, task-scoped spec+quality review, one bounded fix wave/re-review, then one whole-slice final review.

## Required review checkpoint after every Task

After the Task commit and before starting the next Task:

1. Generate a fixed review package from the recorded pre-Task base to the Task head:

```bash
/home/czyt/.cc-switch/skills/subagent-driven-development/scripts/review-package <TASK_BASE> <TASK_HEAD>
```

2. Dispatch one task-scoped reviewer with the extracted Task brief, implementer report, package path, and the Global Constraints above. The reviewer must return both spec-compliance and task-quality verdicts.
3. If Critical/Important findings exist, dispatch exactly one fix wave containing the complete blocking list; the fixer reruns focused tests and appends evidence to the Task report.
4. Re-run exactly one bounded re-review of those findings. Do not expand scope. Minor findings go into `.superpowers/sdd/progress.md` for whole-slice final review.
5. Only after Critical=0 and Important=0 append `Task N: complete (commits ..., review clean)` to the progress ledger and start the next Task.
