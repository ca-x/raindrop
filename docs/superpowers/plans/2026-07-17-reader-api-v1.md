# Raindrop Reader API v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver authenticated Reader entry list/detail APIs plus atomic per-entry read/star state mutation for SQLite, PostgreSQL, and MySQL.

**Architecture:** Keep HTTP wire DTOs in a focused `api::entries` module and reuse the existing user-scoped query repository. Add one short-transaction repository mutation that serializes on the owning Subscription, preserves omitted fields, and stores only sparse read overrides. The database remains the record system; the API never exposes entities or backend errors.

**Tech Stack:** Rust 2024, MSRV 1.94, Axum 0.8.9, SeaORM 1.1.19, Serde 1.0.228, SQLite/PostgreSQL/MySQL.

## Global Constraints

- ASTRYX is not part of this backend slice; the next UI slice consumes this wire contract.
- Rust edition is exactly `2024`; `rust-version` is exactly `1.94`.
- Cargo verification commands use `--locked`.
- No new crate and no database migration.
- HTTP fields use camelCase; enum values use UPPER_SNAKE_CASE.
- All Reader reads require `CurrentUser`; Reader mutation requires `CurrentUser + CsrfGuard`.
- All Reader success and failure responses use `Cache-Control: no-store` and `Pragma: no-cache`.
- All user data queries authorize through the authenticated user's Subscription join.
- Domain DTOs do not gain HTTP Serde derives; wire DTOs map explicitly.
- Only Critical/Important findings block; use one bounded fix wave and one re-review.
- Never modify the user's untracked `.superpowers/research/` or `node_modules/`.

---

## File Map

| File | Responsibility |
| --- | --- |
| `src/api/entries.rs` | Reader routes, query/body extraction, wire DTOs, domain mapping, API error mapping |
| `src/api/error.rs` | Add the uniform Reader `METHOD_NOT_ALLOWED` error |
| `src/api/mod.rs` | Register the entries module |
| `src/api/routes.rs` | Merge the Reader router and expose the shared sensitive cache middleware within `api` |
| `src/feeds/state.rs` | Atomic single-entry state transaction and backend SQL helpers |
| `src/feeds/dto.rs` | `UpdateEntryState` and `EntryStateDto` domain types |
| `src/feeds/mod.rs` | Export state domain types and register the state module |
| `src/feeds/query.rs` | Add the redacted `InvalidStatePatch` repository error variant |
| `.github/workflows/ci.yml` | Run PostgreSQL/MySQL state contracts in existing serial backend steps |
| `tests/reader_api.rs` | Router-level list/detail/PATCH/auth/CSRF/cache contracts |
| `tests/feed_state_contracts.rs` | Repository sparse-state/idempotency/concurrency/backend contracts |

### Task 1: Authenticated Reader list and detail HTTP contract

**Files:**
- Create: `src/api/entries.rs`
- Modify: `src/api/error.rs`
- Modify: `src/api/mod.rs`
- Modify: `src/api/routes.rs`
- Create: `tests/reader_api.rs`

**Interfaces:**
- Consumes: `FeedRepository::list_for_user`, `FeedRepository::get_detail_for_user`, `CurrentUser`, `SetupService::database`, `ApiError`.
- Produces: `pub(super) fn router() -> Router<AppState>` with `GET /api/v1/entries` and `GET /api/v1/entries/{entry_id}`.

- [ ] **Step 1: Write one failing HTTP list tracer test**

Seed a migrated SQLite database with two users, one shared Feed, user-scoped Subscriptions, visible Entries, a sanitized detail envelope, and one sparse star/read state. Create sessions through `SetupService::sessions().create` and send the real cookie.

The first test asserts only list authentication, default UNREAD, one-item cursor pagination, and cache headers:

```rust
#[tokio::test]
async fn reader_list_defaults_to_unread_and_returns_a_user_bound_cursor() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request(Method::GET, "/api/v1/entries?limit=1", None, UserKind::A)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let body = response_json(response).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["isRead"], false);
    assert!(body["nextCursor"].is_string());
    assert_eq!(body["snapshotGeneration"], 1);

    let cursor = body["nextCursor"].as_str().unwrap();
    let response = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?limit=1&cursor={cursor}"),
            None,
            UserKind::B,
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

```

- [ ] **Step 2: Run the Reader HTTP test and verify RED**

Run:

```bash
cargo test --locked --all-features --test reader_api
```

Expected: compilation failure because `api::entries` and Reader routes do not exist, or route assertions return the embedded-web fallback instead of the required JSON contract.

- [ ] **Step 3: Implement only the list query extractor, list wire DTO, and list handler**

Create `src/api/entries.rs` with the following public shape:

```rust
use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::request::Parts,
    middleware,
    routing::get,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    app::AppState,
    auth::CurrentUser,
    feeds::{
        EntryListItemDto, EntryListState, FeedRepository, ListEntriesQuery, RepositoryError,
    },
};

use super::{ApiError, routes::sensitive_cache_headers};

pub(super) fn router() -> Router<AppState> {
    let entries = Router::new().route("/", get(list_entries));
    Router::new()
        .route("/api/v1/entries/", axum::routing::any(reader_not_found))
        .nest("/api/v1/entries", entries)
        .layer(middleware::map_response(sensitive_cache_headers))
}
```

Map Axum query rejection into the uniform envelope:

```rust
struct ApiQuery<T>(T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|_| ApiError::validation())
    }
}
```

Use exact query types:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListEntriesParams {
    cursor: Option<String>,
    limit: Option<u16>,
    feed_id: Option<String>,
    state: Option<EntryStateParam>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum EntryStateParam {
    All,
    Unread,
    Starred,
}
```

Convert missing values to `ListEntriesQuery::default()`. Let the repository validate limit, UUID, and cursor so one canonical validator remains.

Define list wire DTOs with `#[serde(rename_all = "camelCase")]`. `EntryPageResponse` contains `items`, `next_cursor`, and `snapshot_generation`.

Handlers use the live database after setup:

```rust
fn repository(state: &AppState) -> Result<FeedRepository, ApiError> {
    state
        .setup
        .database()
        .map(FeedRepository::new)
        .map_err(|_| ApiError::internal())
}

async fn list_entries(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<ListEntriesParams>,
) -> Result<Json<EntryPageResponse>, ApiError> {
    let page = repository(&state)?
        .list_for_user(&user.id, params.into_query())
        .await
        .map_err(map_repository_error)?;
    Ok(Json(page.into()))
}
```

Map `InvalidUserId | InvalidFeedId | InvalidEntryId | InvalidLimit | InvalidCursor` to `ApiError::validation()` with an appropriate field where useful. Map `Database | CorruptData | Content` to `ApiError::internal()`.

- [ ] **Step 4: Run the list tracer and verify GREEN**

```bash
cargo test --locked --all-features --test reader_api reader_list_defaults
```

- [ ] **Step 5: Add the detail/isolation tracer, verify RED, then implement detail**

Add these exact tests before implementing the detail route:

```rust
#[tokio::test]
async fn reader_detail_returns_only_sanitized_visible_content() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries/{ENTRY_A_ID}"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["contentHtml"], "<p>Safe content</p>");
    assert!(body["inertImages"].is_array());
    assert!(body["enclosures"].is_array());
    assert!(!body.to_string().contains("<script"));
}

#[tokio::test]
async fn guessed_or_invisible_entry_ids_share_the_same_not_found_contract() {
    let fixture = ReaderFixture::new().await;
    let mut envelopes = Vec::new();
    for entry_id in [ENTRY_A_ID, "00000000-0000-4000-8000-000000000399"] {
        let response = fixture
            .request(
                Method::GET,
                &format!("/api/v1/entries/{entry_id}"),
                None,
                UserKind::B,
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        envelopes.push((body["error"]["code"].clone(), body["error"]["message"].clone()));
    }
    assert_eq!(envelopes[0], envelopes[1]);
}
```

Run the two detail tests and observe RED, then add `GET /api/v1/entries/{entry_id}`, `EntryDetailResponse`, and the handler. Convert missing enclosure envelope with `unwrap_or_default()`.

- [ ] **Step 6: Add validation/cache/fallback cases one class at a time**

For each named class, add the test, run its exact filter to observe RED, implement the minimal mapping, then rerun GREEN:

- `reader_list_rejects_invalid_query_contract`: 422 for `limit=0`, `limit=101`, lowercase `state=unread`, malformed `feedId`, malformed cursor, duplicate scalar fields, unknown `categoryId`;
- `reader_routes_require_authentication`: 401 for unauthenticated list/detail;
- `reader_routes_redact_corrupt_persisted_content`: corrupt sanitized/enclosure envelope → generic 500 without stored content leakage;
- `reader_unknown_paths_return_json_not_found`: unknown child and trailing-slash path → JSON 404;
- `reader_known_paths_reject_wrong_methods`: wrong method on a known Reader path → JSON 405 `METHOD_NOT_ALLOWED`;
- `reader_responses_disable_caching`: every case carries both cache headers.

Use this exact command form for each class:

```bash
cargo test --locked --all-features --test reader_api <exact_test_name>
```

Add to `ApiError`:

```rust
pub fn method_not_allowed() -> Self {
    Self::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
        "The request method is not allowed",
    )
}
```

The final Reader router uses an inner scoped fallback, avoiding the Axum conflict between a single-segment capture and a catch-all route:

```rust
let entries = Router::new()
    .route("/", get(list_entries))
    .route("/{entry_id}", get(get_entry))
    .fallback(reader_not_found)
    .method_not_allowed_fallback(reader_method_not_allowed);

Router::new()
    .route("/api/v1/entries/", axum::routing::any(reader_not_found))
    .nest("/api/v1/entries", entries)
    .layer(middleware::map_response(sensitive_cache_headers))
```

Axum 0.8.9 guarantees inner `/` matches the exact nest prefix `/api/v1/entries`, but `/api/v1/entries/` does not enter that nest and must be handled explicitly by the outer Reader router. Unknown children enter the inner fallback instead of the embedded-web fallback. Task 3 can add `/{entry_id}/state` without wildcard conflicts.

- [ ] **Step 7: Register the module and reusable cache middleware**

In `src/api/mod.rs` add:

```rust
mod entries;
```

In `src/api/routes.rs` make the function visible to sibling API modules:

```rust
pub(super) async fn sensitive_cache_headers(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
```

Merge the Reader router in `router()`:

```rust
Router::new()
    .route("/api/v1/bootstrap", get(bootstrap))
    .route_layer(middleware::map_response(sensitive_cache_headers))
    .nest("/api/v1/setup", setup)
    .nest("/api/v1/auth", auth)
    .merge(super::entries::router())
    .layer(DefaultBodyLimit::max(64 * 1024))
```

- [ ] **Step 8: Run focused and related query tests and verify GREEN**

Run:

```bash
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test feed_query_contracts
```

Expected: all tests pass; conditional PostgreSQL/MySQL query tests may report pass after their documented environment skip.

- [ ] **Step 9: Commit Task 1**

```bash
git add src/api/entries.rs src/api/error.rs src/api/mod.rs src/api/routes.rs tests/reader_api.rs
git commit -m "feat: expose reader entry queries"
```

### Task 2: Atomic sparse read/star repository state

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `src/feeds/state.rs`
- Modify: `src/api/entries.rs`
- Modify: `src/feeds/dto.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/feeds/query.rs`
- Create: `tests/feed_state_contracts.rs`

**Interfaces:**
- Consumes: `FeedRepository::connection`, `subscriptions`, `entries`, `entry_states` schema.
- Produces: `FeedRepository::update_state_for_user(&str, &str, UpdateEntryState) -> Result<Option<EntryStateDto>, RepositoryError>`.

- [ ] **Step 1: Write failing repository behavior tests one vertical case at a time**

Start with a single test for read-only mutation and omitted star preservation:

```rust
#[tokio::test]
async fn sqlite_read_patch_uses_sparse_override_and_preserves_star() {
    let fixture = StateFixture::sqlite("read-preserves-star").await;
    fixture.seed_state(Some(false), true, 1).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(true),
                is_starred: None,
            },
        )
        .await
        .expect("state update should remain typed")
        .expect("visible entry should update");

    assert!(state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, Some(true));
    assert!(stored.is_starred);
    assert_eq!(stored.revision, 2);
}
```

Then cycle red→green for:

- base read request stores `read_override=NULL`;
- explicit unread above/below frontier;
- star false→true sets database `starred_at`;
- repeated true preserves timestamp and revision;
- true→false clears timestamp;
- empty patch returns `InvalidStatePatch`;
- malformed UUID returns the typed validation variant;
- absent/cross-user/pre-subscription returns `None` and creates no row;
- first no-op base-read/false-star request does not create a row;
- clearing the last read override deletes the row;
- un-starring a starred-only row deletes the row;
- clearing both fields deletes the row, while clearing one field preserves the other;
- combined patch changes both fields in one revision;
- two SQLite connections concurrently write different fields and final state contains both;
- PostgreSQL/MySQL deterministic barrier contract: T1 holds the Subscription lock, T2 has started its lock attempt, T1 commits, and T2 must observe and preserve T1's field. PostgreSQL uses `pg_stat_activity` for the same database role. MySQL uses `SHOW PROCESSLIST`, filters the same-account row by T2 `CONNECTION_ID()`, and requires a state containing `lock`; it must not require global `PROCESS` or query `information_schema.innodb_trx`.

Use the same contract function for PostgreSQL/MySQL under existing environment-gated serial helpers.

Add explicit CI filters after the existing backend query/terminal filters:

```yaml
- name: Test PostgreSQL reader state contracts
  run: cargo test --locked --test feed_state_contracts postgres -- --nocapture --test-threads=1

- name: Test MySQL reader state contracts
  run: cargo test --locked --test feed_state_contracts mysql -- --nocapture --test-threads=1
```

The workflow's existing masked `RAINDROP_TEST_POSTGRES_URL` and `RAINDROP_TEST_MYSQL_URL` values from `GITHUB_ENV` are reused; do not introduce a second URL source.

- [ ] **Step 2: Run the repository test and verify RED**

```bash
cargo test --locked --all-features --test feed_state_contracts
```

Expected: compilation failure because `UpdateEntryState`, `EntryStateDto`, and `update_state_for_user` do not exist.

- [ ] **Step 3: Add the domain DTOs and error variant**

Append to `src/feeds/dto.rs`:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateEntryState {
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryStateDto {
    pub entry_id: String,
    pub is_read: bool,
    pub is_starred: bool,
}
```

Add `RepositoryError::InvalidStatePatch` and include it in the redacted `Debug` match.

In the same Task 2 cycle, extend `src/api/entries.rs::map_repository_error` with an explicit `InvalidStatePatch => ApiError::validation()` arm so Task 2 remains independently compilable. Do not use a wildcard arm.

Register/export in `src/feeds/mod.rs`:

```rust
mod state;

pub use dto::{
    EnclosureDto, EntryDetailDto, EntryListItemDto, EntryPage, EntryStateDto, InertImageDto,
    RefreshDto, SubscribeInput, SubscriptionDto, UpdateEntryState,
};
```

- [ ] **Step 4: Implement the short transaction and backend lock strategy**

Create `src/feeds/state.rs`. The top-level flow must be exactly:

```rust
impl FeedRepository {
    pub async fn update_state_for_user(
        &self,
        user_id: &str,
        entry_id: &str,
        patch: UpdateEntryState,
    ) -> Result<Option<EntryStateDto>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_uuid(entry_id).map_err(|()| RepositoryError::InvalidEntryId)?;
        if patch.is_read.is_none() && patch.is_starred.is_none() {
            return Err(RepositoryError::InvalidStatePatch);
        }

        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let Some(locked) = lock_visible_subscription(
            &transaction,
            backend,
            user_id,
            entry_id,
        )
        .await?
        else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let current = load_current_state_after_lock(
            &transaction,
            backend,
            &locked,
            user_id,
            entry_id,
        )
        .await?;

        let next = current.apply(patch);
        if next.is_neutral() {
            if current.state_exists {
                delete_state_row(&transaction, backend, &current).await?;
            }
            transaction.commit().await?;
            return Ok(Some(next.dto()));
        }
        if next.same_storage(&current) {
            transaction.commit().await?;
            return Ok(Some(next.dto()));
        }

        if current.state_exists {
            update_state_row(&transaction, backend, &current, &next).await?;
        } else {
            insert_state_row(&transaction, backend, &current, &next).await?;
        }
        transaction.commit().await?;
        Ok(Some(next.dto()))
    }
}
```

`lock_visible_subscription` is the first database step. On SQLite it executes the scoped no-op `UPDATE` below and then reads the matching Subscription ID. On PostgreSQL/MySQL it runs a locking authorization query using this predicate without joining `entry_states`:

```sql
FROM subscriptions s
JOIN entries e ON e.feed_id = s.feed_id
WHERE s.user_id = :user_id
  AND e.id = :entry_id
  AND e.feed_sequence > s.start_sequence
```

Append `FOR UPDATE OF s` on PostgreSQL and `FOR UPDATE` on MySQL. SQLite must execute this parameterized, scoped no-op update as the first transaction statement so it obtains the writer lock before any state read:

```sql
UPDATE subscriptions
SET state_revision = state_revision
WHERE user_id = ?
  AND feed_id = (
    SELECT e.feed_id
    FROM entries e
    WHERE e.id = ?
      AND e.feed_id = subscriptions.feed_id
      AND e.feed_sequence > subscriptions.start_sequence
  )
```

Only after the Subscription lock returns may `load_current_state_after_lock` issue a second statement that reads the current subscription frontier, entry sequence, and optional `entry_states` row. PostgreSQL relies on the second statement's post-wait READ COMMITTED snapshot. MySQL uses a locking/current read for the second statement. SQLite already holds the writer lock. This second query repeats authenticated user, subscription ID, entry ID, feed equality, and `feed_sequence > start_sequence`; it is not an unscoped entry lookup.

The in-memory transition rules are exact:

```rust
let base_read = current.feed_sequence <= current.read_through_sequence;
let read_override = match patch.is_read {
    Some(requested) if requested == base_read => None,
    Some(requested) => Some(requested),
    None => current.read_override,
};
let is_starred = patch.is_starred.unwrap_or(current.is_starred);
let is_read = read_override.unwrap_or(base_read);
```

`next.is_neutral()` means `read_override.is_none() && !is_starred`. If an existing row becomes neutral, delete it with `WHERE user_id = ? AND entry_id = ? AND revision = ?` and require exactly one affected row. Otherwise insert revision `1`, or update with the same revision guard and `revision = revision + 1` only when stored semantics changed. Database clocks are:

```text
SQLite:     strftime('%Y-%m-%dT%H:%M:%f000Z','now')
PostgreSQL: clock_timestamp()
MySQL:      UTC_TIMESTAMP(6)
```

For star transitions, false→true assigns the DB clock, repeated true preserves `starred_at`, and false assigns NULL. SQL text may branch on the trusted patch shape, but all request values remain bound parameters.

- [ ] **Step 5: Run focused state, migration, and ingestion tests and verify GREEN**

```bash
cargo test --locked --all-features --test feed_state_contracts
cargo test --locked --all-features --test rss_migrations
cargo test --locked --all-features --test feed_ingestion_e2e
```

Expected: SQLite behavior/concurrency passes; PostgreSQL/MySQL tests execute when their URLs exist and otherwise use the repository's established documented skip.

- [ ] **Step 6: Commit Task 2**

```bash
git add .github/workflows/ci.yml src/api/entries.rs src/feeds/state.rs src/feeds/dto.rs src/feeds/mod.rs src/feeds/query.rs tests/feed_state_contracts.rs
git commit -m "feat: persist reader entry state"
```

### Task 3: PATCH state HTTP seam and bounded integration gate

**Files:**
- Modify: `src/api/entries.rs`
- Modify: `tests/reader_api.rs`

**Interfaces:**
- Consumes: `FeedRepository::update_state_for_user`, `CurrentUser`, `CsrfGuard`, `ApiJson`.
- Produces: `PATCH /api/v1/entries/{entry_id}/state` with the exact wire contract in the design spec.

- [ ] **Step 1: Add only one failing successful PATCH tracer**

First tracer test:

```rust
#[tokio::test]
async fn reader_state_patch_updates_only_supplied_fields() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request_with_csrf(
            Method::PATCH,
            &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
            json!({ "isStarred": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let body = response_json(response).await;
    assert_eq!(body["entryId"], ENTRY_A_ID);
    assert_eq!(body["isRead"], false);
    assert_eq!(body["isStarred"], true);
}
```

- [ ] **Step 2: Run the PATCH tests and verify RED**

```bash
cargo test --locked --all-features --test reader_api reader_state
```

Expected: 404/405 because the PATCH route is not registered.

- [ ] **Step 3: Implement presence-aware request parsing and PATCH handler**

Use a plain `Option<bool>` with a deserialize function that rejects explicit null while allowing an absent field through `default`:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchEntryStateRequest {
    #[serde(default, deserialize_with = "deserialize_present_bool")]
    is_read: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_present_bool")]
    is_starred: Option<bool>,
}

fn deserialize_present_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bool::deserialize(deserializer).map(Some)
}
```

Register the route:

```rust
.route(
    "/{entry_id}/state",
    axum::routing::patch(patch_entry_state),
)
```

Handler extractor order and behavior are exact:

```rust
async fn patch_entry_state(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(entry_id): Path<String>,
    ApiJson(request): ApiJson<PatchEntryStateRequest>,
) -> Result<Json<EntryStateResponse>, ApiError> {
    if request.is_read.is_none() && request.is_starred.is_none() {
        return Err(ApiError::validation());
    }
    let state = repository(&state)?
        .update_state_for_user(
            &user.id,
            &entry_id,
            UpdateEntryState {
                is_read: request.is_read,
                is_starred: request.is_starred,
            },
        )
        .await
        .map_err(map_repository_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(state.into()))
}
```

Map `InvalidStatePatch` to 422. No new 409 or revision fields are introduced.

- [ ] **Step 4: Run the successful PATCH tracer and verify GREEN**

```bash
cargo test --locked --all-features --test reader_api reader_state_patch_updates_only_supplied_fields
```

- [ ] **Step 5: Add PATCH validation and security classes as separate RED→GREEN cycles**

Add and run one named class at a time through `build_router`:

1. `reader_state_rejects_invalid_bodies`: empty `{}`, explicit `null`, number/string boolean, unknown field, malformed JSON, wrong content type → 422;
2. `reader_state_hides_missing_and_cross_tenant_entries`: malformed path UUID → 422; missing/cross-tenant/pre-subscription entry → identical 404;
3. `reader_state_requires_active_session`: unauthenticated/expired/disabled session → 401;
4. `reader_state_requires_valid_csrf`: missing, duplicate, malformed, mismatched CSRF → 403;
5. `reader_state_enforces_same_origin`: matching Origin/Host succeeds; wrong hostname/port, malformed/duplicate Origin, missing Host with Origin → 403;
6. `reader_state_responses_disable_caching`: every success/error has no-store/no-cache.

For each numbered class: add its test(s), run an exact test-name filter and observe RED, make only the required handler/request-fixture change, then rerun GREEN before adding the next class.

- [ ] **Step 6: Run focused, security, and regression tests**

```bash
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features --test feed_state_contracts
cargo test --locked --all-features --test session_security
cargo test --locked --all-features --test setup_auth_api
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test feed_query_contracts
```

Expected: all pass with zero failures.

- [ ] **Step 7: Run fresh completion gates**

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
git status --short
```

Expected: format/clippy/tests/diff pass. `git status --short` may contain only this task's intended tracked changes plus the user's pre-existing untracked `.superpowers/research/` and `node_modules/`.

- [ ] **Step 8: Commit Task 3**

```bash
git add src/api/entries.rs tests/reader_api.rs docs/superpowers/specs/2026-07-17-reader-api-v1-design.md docs/superpowers/plans/2026-07-17-reader-api-v1.md
git commit -m "feat: add reader state api"
```

### Task 4: Bounded final review, ledger, and push

**Files:**
- Modify: `.superpowers/sdd/progress.md` (ignored durable session ledger only)

**Interfaces:**
- Consumes: commits from Tasks 1–3 and their reports.
- Produces: Critical 0 / Important 0 final verdict and remote branch containing the Reader API v1 commits.

- [ ] **Step 1: Generate one whole-slice review package**

Record the Reader slice base before Task 1 (`fd15df7`) and run:

```bash
bash /home/czyt/.cc-switch/skills/subagent-driven-development/scripts/review-package fd15df7 HEAD
```

Pass the printed file path, the design spec, this plan, and exact Global Constraints to one final reviewer.

- [ ] **Step 2: Apply at most one bounded Critical/Important fix wave**

If the reviewer reports Critical/Important findings, dispatch one fixer with the complete list, require focused tests and output, then run one re-review. Record Minor findings in the progress ledger; do not start an unbounded review loop.

- [ ] **Step 3: Re-run fresh final gates after the last fix**

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
git status --short --branch
```

- [ ] **Step 4: Update ledger and push**

Append:

```text
Reader API v1: complete (commits fd15df7..<head>, review Critical 0 / Important 0; focused state/API contracts and full suite green)
```

Then push:

```bash
git push origin feature/foundation-bootstrap
```

Expected: local HEAD equals `origin/feature/foundation-bootstrap`.

## Self-Review Record

- Spec coverage: every endpoint, DTO, error, auth, CSRF, cache, transaction, isolation, test, review, commit, and push requirement maps to a task.
- Forbidden-marker scan: no deferred implementation markers or unspecified test/error steps remain.
- Type consistency: `UpdateEntryState`, `EntryStateDto`, `RepositoryError::InvalidStatePatch`, `ApiQuery`, and `EntryStateResponse` use one spelling across tasks.
- Pre-flight conflicts: none. Task 1 does not reference Task 2 interfaces; Task 3 consumes Task 2 after its reviewed commit.
