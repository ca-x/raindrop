# Reader Efficiency v2 Implementation Plan

> **For agentic workers:** Execute inline in the main Agent only. Do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add stable-snapshot bulk mark-read, rendered-order unread Feed navigation, and portable Feed-local content search to the existing ASTRYX Reader.

**Architecture:** Extend the Reader OpenAPI and normalized frontend state, add a bounded `entries.search_text` projection, and implement one User-then-Subscription transaction that advances sparse read frontiers. Search stays Feed-local and backend-portable; navigation remains client-derived from exact unread counts.

**Tech Stack:** Rust 1.94.0 edition 2024, Axum, SeaORM/SeaQuery, SQLite/PostgreSQL/MySQL, React 19, TypeScript 7, ASTRYX 0.1.6, Lingui, Vitest, Playwright.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-reader-efficiency-v2-design.md` exactly.
- Main Agent only; do not use subagents.
- No new Rust or npm dependency.
- Bulk mark-read must not insert one state row per Entry.
- Search is Feed-local only and uses the bounded Rust-generated projection.
- Reader wire DTOs come only from committed OpenAPI and generated TypeScript.
- Prefer ASTRYX components and keep feature TypeScript modules focused; avoid a large controller or toolbar file.
- Do not modify `.superpowers/research/` or root `node_modules/`.
- Commit and push each completed task to `feature/foundation-bootstrap`.

---

### Task 1: Search projection schema and deterministic builder

**Files:**
- Create: `src/content/search.rs`
- Modify: `src/content/mod.rs`
- Create: `src/db/migration/rss/entry_search.rs`
- Modify: `src/db/migration/rss/mod.rs`
- Modify: `src/db/migration.rs`
- Modify: `src/db/entities/entry.rs`
- Modify: `src/feeds/persistence.rs`
- Modify: `tests/support/database.rs`
- Modify: `tests/feed_retention_contracts.rs`
- Modify: `tests/feed_subscription_contracts.rs`
- Modify: `tests/rss_migrations.rs`
- Modify: `tests/feed_entry_persistence.rs`

**Interfaces:**
- Produces `build_entry_search_text(title, author, summary, content_html) -> String`.
- Produces `normalize_search_query(raw) -> Result<NormalizedSearch, SearchQueryError>`.
- Produces `entries.search_text` with a 60 KiB projection and 32-row re-entrant backfill.

- [x] Write RED unit tests for rendered text extraction, Unicode lowercase, whitespace collapse, field priority, literal `%/_`, duplicate terms, eight-term limit, 128-byte query limit, and UTF-8-safe 60 KiB truncation.
- [x] Write migration RED tests requiring the column, deterministic backfill, more than one keyset batch, migration-marker re-entry, and rollback.
- [x] Implement the content search builder and typed query normalizer without adding dependencies.
- [x] Add the schema migration, register it after existing Entry storage migrations, and add the entity field.
- [x] Populate `search_text` for new Entries and every metadata/envelope/hash update path. Include it in existing-entry load comparisons so projection repair is deterministic.
- [x] Update shared test ActiveModels and persistence assertions.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked content::search
cargo test --locked --test rss_migrations sqlite_rss_schema_contract -- --nocapture
cargo test --locked --test feed_entry_persistence search_text -- --nocapture
git diff --check
```

- [x] Commit and push: `feat: project searchable entry text`.

### Task 2: Feed-local search repository and API contract

**Files:**
- Modify: `src/feeds/query.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/api/entries.rs`
- Modify: `docs/openapi/reader-v1.json`
- Modify: `tests/reader_api.rs`
- Modify: `tests/reader_openapi_contract/document.rs`
- Modify: `tests/reader_openapi_contract/surface_tests.rs`
- Modify: `tests/reader_openapi_contract/router_tests.rs`
- Modify: `tests/reader_openapi_contract/schema_tests.rs`
- Modify: `tests/reader_openapi_contract/request_tests.rs`
- Modify: `web/src/features/reader/api/reader.generated.ts`

**Interfaces:**
- Extends `ListEntriesQuery` with canonical optional search terms.
- Extends `GET /api/v1/entries` with strict Feed-only `search`.
- Cursor filter hash includes the canonical search term sequence.

- [x] Write RED repository/API tests for title/author/summary/rendered content, multi-term AND, Unicode, literal wildcard characters, Feed requirement, user isolation, cursor mismatch, and invalid byte/term bounds.
- [x] Extend `ListEntriesParams`, repository validation, filter hash, and backend statement generation using `instr`/`position`/`locate`.
- [x] Update OpenAPI parameters and strict router/DTO/schema contracts without exposing `search_text`.
- [x] Regenerate committed Reader TypeScript DTOs and require drift checks to pass.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked --test reader_api search -- --nocapture
cargo test --locked --test reader_openapi_contract
cd web
npm run generate:reader-types
npm run check:reader-types
npm run typecheck
cd ..
git diff --check
```

- [x] Commit and push: `feat: search entries within a feed`.

### Task 3: Stable-snapshot bulk mark-read transaction and endpoint

**Files:**
- Create: `src/feeds/bulk_read.rs`
- Modify: `src/feeds/mod.rs`
- Modify: `src/api/entries.rs`
- Modify: `docs/openapi/reader-v1.json`
- Create: `tests/reader_bulk_read.rs`
- Modify: `tests/reader_openapi_contract/document.rs`
- Modify: `tests/reader_openapi_contract/surface_tests.rs`
- Modify: `tests/reader_openapi_contract/router_tests.rs`
- Modify: `tests/reader_openapi_contract/schema_tests.rs`
- Modify: `tests/reader_openapi_contract/request_tests.rs`
- Modify: `web/src/features/reader/api/reader.generated.ts`

**Interfaces:**
- Produces `MarkReadScope`, `MarkReadResult`, and `FeedRepository::mark_read_for_user`.
- Produces strict `POST /api/v1/entries/mark-read -> 204`.

- [ ] Write SQLite-always and environment-gated PostgreSQL/MySQL RED contracts for All/Feed/Category scopes, later generation preservation, frontier advancement, override deletion/clearing, star preservation, empty/no-owned scopes, idempotency, and individual-state concurrency.
- [ ] Write endpoint RED tests for strict body/content type, UUIDs, mutual exclusion, future snapshot, authentication, CSRF, duplicate headers, cache headers, route/method fallbacks, and database failure mapping.
- [ ] Implement User lock, deterministic Subscription locks, per-Subscription snapshot frontier calculation, sparse normalization, conditional revision update, and atomic commit.
- [ ] Add Axum route/request mapping and OpenAPI 204/error contract.
- [ ] Regenerate Reader types and verify no internal frontier/revision fields appear.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked --test reader_bulk_read -- --nocapture --test-threads=1
cargo test --locked --test reader_api mark_read -- --nocapture
cargo test --locked --test reader_openapi_contract
cd web
npm run generate:reader-types
npm run check:reader-types
cd ..
git diff --check
```

- [ ] Commit and push: `feat: mark reader snapshots read`.

### Task 4: Reader snapshots, search, bulk action, and unread navigation

**Files:**
- Modify: `web/src/features/reader/api/entries.ts`
- Modify: `web/src/features/reader/api/entries.test.ts`
- Modify: `web/src/features/reader/model/types.ts`
- Modify: `web/src/features/reader/model/reducer.ts`
- Modify: `web/src/features/reader/model/reducerEntries.ts`
- Modify: `web/src/features/reader/model/useReaderRequests.ts`
- Modify: `web/src/features/reader/model/useReaderController.ts`
- Create: `web/src/features/reader/model/useBulkReadActions.ts`
- Create: `web/src/features/reader/model/unreadSourceNavigation.ts`
- Create: `web/src/features/reader/model/unreadSourceNavigation.test.ts`
- Modify: `web/src/features/reader/model/reducer.requests.test.ts`
- Modify: `web/src/features/reader/model/useReaderController.test.tsx`
- Modify: `web/src/features/reader/model/useReaderController.mutations.test.tsx`
- Modify: `web/src/features/reader/keyboard/useReaderHotkeys.ts`
- Modify: `web/src/features/reader/keyboard/useReaderHotkeys.test.tsx`

**Interfaces:**
- Adds visible and pending snapshot maps plus one active Feed search query.
- Adds `searchFeed`, `markCurrentSourceRead`, `nextUnreadSource`, and `previousUnreadSource` controller actions.
- Adds Shift+J/K with existing modal/editable guards.

- [ ] Write reducer RED tests proving replace stores a snapshot, discover parks a newer snapshot, merge promotes it, and source/search changes reject late pages.
- [ ] Write controller/API RED tests for Feed search submit/clear, search clearing on source change, 204 mark-read, post-mutation subscription/source reload, and failure rollback/error feedback.
- [ ] Write navigation RED tests for rendered category/uncategorized order, Feed/Category/smart starting points, zero counts, forward/backward boundaries, and UNREAD fallback.
- [ ] Extend API/controller/reducer in focused modules, keeping generated DTOs authoritative.
- [ ] Add Shift+J/K and update the immediate modal key guard.
- [ ] Verify focused Vitest, typecheck, and generated-contract checks.
- [ ] Commit and push: `feat: navigate and filter reader sources`.

### Task 5: ASTRYX UI, i18n, mobile, and final verification

**Files:**
- Create: `web/src/features/reader/components/FeedSearchInput.tsx`
- Create: `web/src/features/reader/components/MarkReadDialog.tsx`
- Split/Modify: `web/src/features/reader/components/ReaderToolbar.tsx`
- Modify: `web/src/features/reader/components/EntryQueue.tsx`
- Modify: `web/src/features/reader/layout/ReaderShell.tsx`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: `web/src/shared/i18n/locales/en/messages.po`
- Modify: `web/src/shared/i18n/locales/zh-CN/messages.po`
- Modify: `web/src/features/reader/ReaderWorkspace.test.tsx`
- Modify: `web/src/features/reader/ReaderKeyboardWorkspace.test.tsx`
- Modify: `web/e2e/reader-workspace.spec.ts`
- Modify: `web/e2e/support/readerApiFixture.ts`
- Modify: `tasks/todo.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/reader-efficiency-v2-report.md`

**Interfaces:**
- Uses ASTRYX `TextInput`, `AlertDialog`, `Toolbar`, `Button`, `MoreMenu`, `Kbd`, and existing responsive shell components.
- Produces responsive search/mark/navigation controls and browser evidence.

- [ ] Write UI RED tests for Feed-only search visibility, Enter/clear behavior, 128-byte error, Starred/search mark-read boundary, snapshot confirmation copy, loading/focus behavior, and compact action access.
- [ ] Implement focused search and confirmation components; split toolbar actions rather than growing one large TSX file.
- [ ] Add complete English/Chinese copy and preserve screen-reader labels/keyboard hints.
- [ ] Add Playwright scenarios at 1280x800, 900x800, 390x844, and 360x800 for search, mark-read pending-snapshot protection, Shift+J/K, modal focus, and horizontal overflow.
- [ ] Run local production build with the real `https://www.ithome.com/rss/` subscription and verify the flow through local `agent-browser`.
- [ ] Apply `$find-animation-opportunities` read-only audit after the feature is complete, implement only justified motion if any, then apply `$kill-ai-slop` cleanup and rerun browser checks.
- [ ] Run full gates:

```bash
cargo fmt --check
cargo test --locked --all-features
cd web
npm run check:reader-types
npm run typecheck
npm run test:ci
npm run build
npm run test:e2e
cd ..
git diff --check
```

- [ ] Mark `CommaFeed 后续：批量已读快照、下一未读来源和来源内搜索` complete and record exact verification/CI evidence in the report.
- [ ] Commit and push: `test: verify reader efficiency workflows`.

## Plan self-review

- Spec coverage: schema/backfill, portable search, cursor binding, sparse bulk transaction, strict OpenAPI, snapshot state, navigation, ASTRYX/mobile UI, i18n, browser, real-feed verification, motion audit, and AI-slop cleanup each map to a task.
- Dependency order: projection schema -> search API -> bulk transaction -> controller state -> UI/browser.
- DDIA consistency: database locks and monotonic generations are authoritative; write amplification is bounded by Subscriptions and sparse exceptions; search storage is capped and backend-neutral.
- API consistency: strict request fields, 204 response, no source disclosure, generated TypeScript, and stable error/security headers are explicit.
- UI consistency: Feed search and mark-read boundaries prevent accidental scope expansion; pending snapshot promotion is explicit; Shift+J/K uses rendered source order.
- Placeholder scan: no TBD, unspecified error behavior, or unbounded review step remains.
- Scope exclusions: Starred/search-match mark-read, global/Category search, relevance ranking, saved search, AI/plugin/MCP content search, and pagination redesign remain separate work.
