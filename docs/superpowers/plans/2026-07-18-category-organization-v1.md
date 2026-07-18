# Raindrop Category Organization v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This project is explicitly configured for inline main-Agent execution.

**Goal:** Add user-scoped one-level categories, subscription assignment, category Reader filters, and an ASTRYX category management flow.

**Architecture:** A new `organization` domain owns category records and transactions. Subscription membership remains on the existing subscription row, Reader queries extend their signed filter contract with `categoryId`, and the frontend adds categories to the existing generated clients, reducer, routes, and TreeList rather than creating a second store.

**Tech Stack:** Rust 2024, Axum, SeaORM/SeaORM Migration, SQLite/PostgreSQL/MySQL, React 19, TypeScript, React Router 7, ASTRYX 0.1.6, Lingui, Vitest, Playwright.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-category-organization-v1-design.md` exactly.
- No new runtime dependency.
- Every repository query accepts an explicit user ID; cross-user IDs return 404 or an empty list without revealing existence.
- TypeScript wire DTOs come only from committed OpenAPI artifacts.
- Use ASTRYX components before custom markup; no custom dialog, selector, list, or tree control.
- Keep new non-generated TS/TSX and Rust modules focused; split before a file exceeds roughly 250 lines where the existing architecture permits.
- Do not touch `.superpowers/research/` or root `node_modules/`.
- All implementation and review are performed inline by the main Agent; do not dispatch subagents.

---

### Task 1: Category schema and migration contract

**Files:**
- Create: `src/db/migration/organization.rs`
- Create: `src/db/entities/category.rs`
- Create: `tests/organization_migrations.rs`
- Modify: `src/db/migration.rs`
- Modify: `src/db/entities.rs`
- Modify: `src/db/entities/subscription.rs`
- Modify: `tests/support/database.rs`

**Interfaces:**
- Produces `categories(id, user_id, title, normalized_title, position, created_at, updated_at)` and nullable `subscriptions.category_id`.
- Produces `category::Model` and additive `subscription::Model.category_id: Option<String>` used by all later tasks.

- [x] **Step 1: Write the migration RED contract**

Add a SQLite contract that migrates twice, asserts the category indexes/foreign keys, inserts two users and categories, rejects duplicate normalized titles per user, assigns a subscription, deletes the category, and proves the subscription survives with `category_id = NULL`. Include opt-in PostgreSQL/MySQL entry points using `RAINDROP_TEST_POSTGRES_URL` and `RAINDROP_TEST_MYSQL_URL`.

```rust
#[tokio::test]
async fn sqlite_organization_schema_contract() {
    let url = sqlite_contract_url("organization-contract.db");
    organization_schema_contract(url).await;
}
```

- [x] **Step 2: Run the focused test and confirm RED**

Run: `cargo test --locked --all-features --test organization_migrations`

Expected: compile failure because `entities::category` and `subscription.category_id` do not exist.

- [x] **Step 3: Implement the additive migration and entities**

Append `CreateOrganizationTables` after the current RSS migrations. Create category indexes `uq_categories_user_normalized_title` and `idx_categories_user_position`; add `category_id`, FK `fk_subscriptions_category` with `ON DELETE SET NULL`, and `idx_subscriptions_user_category_position`.

```rust
pub struct Model {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub normalized_title: String,
    pub position: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
```

- [x] **Step 4: Update shared fixtures and all ActiveModels**

Set `category_id: Set(None)` in `tests/support/database.rs` and every direct `subscription::ActiveModel` literal found by `rg -n "subscription::ActiveModel" src tests`.

- [x] **Step 5: Verify migration contracts**

Run: `cargo test --locked --all-features --test organization_migrations --test rss_migrations --test database_migrations`

Expected: all tests pass; external database contracts skip only when their URLs are absent.

- [x] **Step 6: Commit and push**

```bash
git add src/db/migration.rs src/db/migration/organization.rs src/db/entities.rs src/db/entities/category.rs src/db/entities/subscription.rs tests/organization_migrations.rs tests/support/database.rs
git commit -m "feat: add category storage"
git push origin feature/foundation-bootstrap
```

### Task 2: User-scoped category repository

**Files:**
- Create: `src/organization/mod.rs`
- Create: `src/organization/category.rs`
- Create: `tests/category_repository.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces `CategoryRepository::list/create/update/delete`.
- Produces `CategoryDto`, `CreateCategory`, `UpdateCategory`, and `CategoryError` for the API layer.

- [x] **Step 1: Write repository RED tests**

Cover title normalization, control-character rejection, 80-scalar/200-byte bounds, stable `(position,id)` order, per-user duplicate isolation, 250-category quota, cross-user update/delete as `NotFound`, delete set-null behavior, and concurrent duplicate create yielding exactly one row.

```rust
let result = repository
    .create(USER_A_ID, CreateCategory { title: " Tech ".into() })
    .await?;
assert_eq!(result.title, "Tech");
```

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test category_repository`

Expected: compile failure because `raindrop::organization` does not exist.

- [x] **Step 3: Implement validation and redacted errors**

Use a private `NormalizedCategoryTitle { display, normalized }`. Reject C0/C1 controls, count Unicode scalars, bound UTF-8 bytes, and never include raw titles in `Debug`/`Display` errors.

- [x] **Step 4: Implement transaction methods**

Lock the active user row, enforce quota/exact conflicts inside the transaction, assign `max(position) + 1024`, scope update/delete by both category ID and user ID, and rely on the FK for delete set-null.

- [x] **Step 5: Verify repository and migration contracts**

Run: `cargo test --locked --all-features --test category_repository --test organization_migrations`

- [x] **Step 6: Commit and push**

```bash
git add src/lib.rs src/organization/mod.rs src/organization/category.rs tests/category_repository.rs
git commit -m "feat: add category repository"
git push origin feature/foundation-bootstrap
```

### Task 3: Category HTTP and OpenAPI contract

**Files:**
- Create: `src/api/categories.rs`
- Create: `docs/openapi/organization-v1.json`
- Create: `tests/category_api.rs`
- Create: `tests/organization_openapi_contract.rs`
- Modify: `src/api/mod.rs`
- Modify: `src/api/routes.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Produces real `GET/POST/PATCH/DELETE /api/v1/categories` routes.
- Produces the committed organization artifact consumed by the frontend generator.

- [x] **Step 1: Write router RED tests**

Test session/CSRF precedence, unknown fields, empty patch, title/position validation, create 201 plus Location, duplicate 409, quota 409, cross-user 404, DELETE 204, no-store headers, trailing slash fallback, and method fallback.

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test category_api`

Expected: category routes return the SPA fallback or 404.

- [x] **Step 3: Implement the category router**

Use `CurrentUser`, `CsrfGuard`, `ApiJson`, `ApiError`, and a dedicated `organization_mutation_limiter`. Map repository errors only to stable public codes; database details remain `INTERNAL_ERROR`.

- [x] **Step 4: Add the OpenAPI artifact and drift test**

The artifact declares exactly four category operations, camelCase DTOs, session/CSRF security, 200/201/204/401/403/404/409/422/429/500 responses, and no internal normalized title/timestamps/user ID.

- [x] **Step 5: Verify router and artifact together**

Run: `cargo test --locked --all-features --test category_api --test organization_openapi_contract`

- [x] **Step 6: Commit and push**

```bash
git add src/api/categories.rs src/api/mod.rs src/api/routes.rs src/app.rs docs/openapi/organization-v1.json tests/category_api.rs tests/organization_openapi_contract.rs
git commit -m "feat: expose category api"
git push origin feature/foundation-bootstrap
```

### Task 4: Subscription assignment and category Reader filter

**Files:**
- Modify: `src/feeds/dto.rs`
- Modify: `src/feeds/subscription.rs`
- Modify: `src/feeds/query.rs`
- Modify: `src/api/subscriptions.rs`
- Modify: `src/api/entries.rs`
- Modify: `docs/openapi/subscription-v1.json`
- Modify: `docs/openapi/reader-v1.json`
- Modify: `tests/subscription_api.rs`
- Modify: `tests/reader_api.rs`
- Modify: `tests/openapi_contract.rs`
- Modify: `tests/reader_openapi_contract/`

**Interfaces:**
- Produces `UpdateSubscription { category_id: PatchValue<String>, title_override: PatchValue<String>, position: Option<i64> }`.
- Extends `Subscription` with required nullable `categoryId`, required nullable `titleOverride`, and required `position`.
- Extends `ListEntriesQuery` with `category_id: Option<String>`.

- [x] **Step 1: Write subscription/category query RED tests**

Cover assign, clear, title override, position, empty patch, other-user category 404, other-user subscription 404, delete category set-null projection, category list results, feed/category mutual exclusion, category cursor replay rejection, and cross-user category returning an empty page.

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test subscription_api --test reader_api`

- [x] **Step 3: Implement patch presence semantics**

Use a deserializable `PatchValue<T>` enum so absent and explicit null remain distinct. Scope the subscription update by user ID and validate a non-null category in the same transaction.

- [x] **Step 4: Extend query SQL and cursor binding**

Add category to the filter hash frame and SQL predicate. Reject simultaneous feed/category before any database query.

- [x] **Step 5: Update both OpenAPI artifacts and frozen manifests**

Add the PATCH operation and additive Subscription fields to `subscription-v1.json`; add `CategoryId` to `reader-v1.json`. Update the real-router drift fixtures, not only static schema assertions.

- [x] **Step 6: Verify backend contracts**

Run: `cargo test --locked --all-features --test subscription_api --test reader_api --test openapi_contract --test reader_openapi_contract --test feed_query_contracts --test feed_subscription_contracts`

- [x] **Step 7: Commit and push**

```bash
git add src/feeds/dto.rs src/feeds/subscription.rs src/feeds/query.rs src/api/subscriptions.rs src/api/entries.rs docs/openapi/subscription-v1.json docs/openapi/reader-v1.json tests/subscription_api.rs tests/reader_api.rs tests/openapi_contract.rs tests/reader_openapi_contract tests/feed_query_contracts.rs tests/feed_subscription_contracts.rs
git commit -m "feat: organize subscriptions by category"
git push origin feature/foundation-bootstrap
```

### Task 5: Generated clients and normalized category state

**Files:**
- Modify: `web/scripts/generate-reader-types.mjs`
- Create: `web/src/features/reader/api/organization.generated.ts`
- Create: `web/src/features/reader/categories/api.ts`
- Create: `web/src/features/reader/categories/api.test.ts`
- Modify: `web/src/features/reader/api/subscriptions.ts`
- Modify: `web/src/features/reader/api/entries.ts`
- Modify: `web/src/features/reader/model/controllerApi.ts`
- Modify: `web/src/features/reader/model/types.ts`
- Modify: `web/src/features/reader/model/reducer.ts`
- Modify: `web/src/features/reader/model/useReaderRequests.ts`
- Add focused reducer/controller tests.

**Interfaces:**
- Extends `ReaderSource` with `{ kind: "category"; categoryId: string }` and `SourceKey` with `category:${string}`.
- Extends `ReaderState` with normalized `categoriesById` and `categoryOrder`.
- Produces category CRUD and subscription patch functions in `ReaderApi`.

- [x] **Step 1: Add generator/client RED tests**

Check that `organization.generated.ts` is absent/stale, malformed category responses fail closed, PATCH sends CSRF, and `listEntries({ categoryId })` serializes only the category filter.

- [x] **Step 2: Run and confirm RED**

Run: `cd web && npm run test:ci -- categories/api.test.ts subscriptions.test.ts entries.test.ts`

- [x] **Step 3: Extend generation and strict validators**

Add `docs/openapi/organization-v1.json` to the artifact list, regenerate all clients, and implement API functions using the existing `apiRequest`/`invalidResponseError` boundary.

- [x] **Step 4: Extend the single reducer/controller**

Load categories and subscriptions together, reject late generations, preserve the selected route, and update subscription/category entities after mutations without introducing a second store.

- [x] **Step 5: Verify generated contracts and state tests**

Run: `cd web && npm run generate:reader-types && npm run check:reader-types && npm run typecheck && npm run test:ci`

- [x] **Step 6: Commit and push**

```bash
git add web/scripts/generate-reader-types.mjs web/src/features/reader/api web/src/features/reader/categories web/src/features/reader/model
git commit -m "feat: model reader categories"
git push origin feature/foundation-bootstrap
```

### Task 6: Category routes and ASTRYX management UI

**Files:**
- Create: `web/src/features/reader/categories/CategoryDialog.tsx`
- Create: `web/src/features/reader/categories/CategoryList.tsx`
- Create: `web/src/features/reader/categories/groupSubscriptions.ts`
- Create focused tests beside those modules.
- Modify: `web/src/features/reader/components/SourceTree.tsx`
- Modify: `web/src/features/reader/components/ReaderToolbar.tsx`
- Modify: `web/src/features/reader/routes/readerRoute.ts`
- Modify: `web/src/features/reader/routes/ReaderRoutes.tsx`
- Modify: `web/src/features/reader/layout/ReaderShell.tsx`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: `web/src/features/reader/reader.css` only for domain layout ASTRYX cannot express.

**Interfaces:**
- Produces `/reader/category/:categoryId` and `/entry/:entryId` parsing/path generation.
- Produces one category management Dialog and TreeList category branches.

- [x] **Step 1: Inspect exact ASTRYX APIs**

Run `node web/node_modules/@astryxdesign/core/docs.mjs` for `Dialog`, `Selector`, `List`, `Item`, and `AlertDialog` before implementation.

- [x] **Step 2: Write route/grouping/Dialog RED tests**

Test encoded IDs, invalid paths, categorized/uncategorized grouping, empty categories, aggregate unread counts, create/rename/delete, assignment/clear, mutation errors, focus return, and deletion of the active category navigating to unread.

- [x] **Step 3: Implement category routes and grouping**

Keep smart/feed/category sources as a discriminated union. TreeList items use category nodes with Feed children and one Uncategorized node; no custom tree DOM.

- [x] **Step 4: Implement the focused ASTRYX Dialog**

Use `Dialog purpose="form"`, `TextInput`, `Selector`, `List/Item`, `Button`, and `AlertDialog`. Do not add drag/drop, nested dialogs, hover scale, or custom animation.

- [x] **Step 5: Verify frontend behavior**

Run: `cd web && npm run check:reader-types && npm run typecheck && npm run test:ci && npm run build`

- [x] **Step 6: Commit and push**

```bash
git add web/src/features/reader/categories web/src/features/reader/components/SourceTree.tsx web/src/features/reader/components/ReaderToolbar.tsx web/src/features/reader/routes web/src/features/reader/layout/ReaderShell.tsx web/src/shared/i18n/i18n.ts web/src/features/reader/reader.css
git commit -m "feat: add category reader workflow"
git push origin feature/foundation-bootstrap
```

### Task 7: Four-viewport browser and live-feed verification

**Files:**
- Modify: `web/e2e/support/readerApiFixture.ts`
- Modify: `web/e2e/support/readerAssertions.ts`
- Modify: `web/e2e/reader-workspace.spec.ts`
- Create: `web/e2e/support/readerOrganizationFixture.ts`
- Create: `web/e2e/support/readerCategoryScenarios.ts`
- Modify: `web/src/features/reader/components/ReaderToolbar.tsx`
- Modify: `web/src/shared/brand/BrandMark.test.tsx`
- Modify: `web/src/features/auth/LoginPage.test.tsx`
- Modify: `tasks/todo.md`
- Create: `.superpowers/sdd/category-organization-v1-report.md`

**Interfaces:**
- Produces deterministic browser evidence and updates the authoritative task state.

- [x] **Step 1: Extend the stateful Reader fixture**

Support category CRUD, subscription PATCH, category-filtered entry pages, category deletion set-null, and cross-user denial while keeping bootstrap/session/CSRF/production assets real.

- [x] **Step 2: Add Playwright scenarios**

At 1280×800 create/rename/assign/navigate/delete; at 900×800 verify drawer focus; at 390×844 and 360×800 verify category deep link, UI/browser Back, Dialog containment, and no horizontal overflow.

- [x] **Step 3: Run local agent-browser with the real IT Home subscription**

Use `https://www.ithome.com/rss/`, create a category, assign the subscription, open a category article, reload, and prove the assignment/filter persists. Record sanitized console/error/viewport evidence without cookies or database files.

- [x] **Step 4: Run full gates once**

Run the Reader type drift check, TypeScript, Vitest, production build, Playwright, full locked Rust suite, and `git diff --check` once after the implementation stabilizes.

- [x] **Step 5: Update task/report state**

Mark categories complete while leaving user settings, admin management, OIDC, OPML, AI/plugin/MCP and release unchecked. Record exact test counts and existing chunk/future-incompatibility advisories.

- [x] **Step 6: Commit and push**

```bash
git add web/e2e/reader-workspace.spec.ts web/e2e/support/readerApiFixture.ts web/e2e/support/readerAssertions.ts web/e2e/support/readerOrganizationFixture.ts web/e2e/support/readerCategoryScenarios.ts web/src/features/reader/components/ReaderToolbar.tsx web/src/shared/brand/BrandMark.test.tsx web/src/features/auth/LoginPage.test.tsx tasks/todo.md docs/superpowers/plans/2026-07-18-category-organization-v1.md
git add -f .superpowers/sdd/category-organization-v1-report.md
git commit -m "test: verify category organization"
git push origin feature/foundation-bootstrap
```

## Plan self-review

- Spec coverage: schema, concurrency, CRUD, assignment, category filter, cursor binding, OpenAPI generation, routes, ASTRYX UI, mobile, real Feed and cross-user denial each map to a task.
- Dependency order: schema → repository → category API → subscription/query integration → generated state → UI → browser evidence.
- Type consistency: `categoryId`, `titleOverride`, `position`, `ReaderSource.kind = "category"`, and `category:${string}` are identical across backend DTO, OpenAPI, generated types and frontend state.
- Placeholder scan completed: no incomplete marker, deferred implementation instruction, generic error-handling step, or undefined later interface remains.
- Scope exclusions: user settings, registration/admin, OIDC, OPML, AI/plugin/MCP and release stay outside this plan and remain explicit backlog items.
