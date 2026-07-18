# Refresh Observability v1 Implementation Plan

> **For agentic workers:** Execute inline in the main Agent only. Do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Distinguish queued and running refreshes, expose last success and bounded entry issues, and present cooldown/partial feedback in the responsive ASTRYX Reader.

**Architecture:** Extend the existing `Refresh` projection without adding storage. Rust derives `pendingState` and `entryIssues` from authoritative run fields and joins `feeds.last_success_at`; the Web client consumes generated DTOs through a focused refresh-presentation module and one selected-Feed status component.

**Tech Stack:** Rust 1.94.0 edition 2024, Axum, SeaORM/SeaQuery, SQLite/PostgreSQL/MySQL, React 19, TypeScript 7, ASTRYX 0.1.6, Lingui, Vitest, Playwright.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-refresh-observability-v1-design.md` exactly.
- Main Agent only; do not use subagents.
- No new database table, Rust dependency, or npm dependency.
- Existing `Refresh.state` values remain unchanged.
- Reader wire DTOs come only from committed OpenAPI and generated TypeScript.
- Use existing feature-scoped `web/src/features/reader/reader.css`; do not add another styling system.
- Prefer ASTRYX components and keep refresh presentation in focused modules.
- Do not modify `.superpowers/research/` or root `node_modules/`.
- Commit and push each independently verified task to `feature/foundation-bootstrap`.

---

### Task 1: Authoritative refresh projection and public contract

**Files:**
- Modify: `src/feeds/dto.rs`
- Modify: `src/feeds/subscription.rs`
- Modify: `src/api/subscriptions.rs`
- Modify: `docs/openapi/subscription-v1.json`
- Modify: `tests/feed_subscription_contracts.rs`
- Modify: `tests/subscription_api.rs`
- Modify: `tests/openapi_contract.rs`
- Modify: `web/src/features/reader/api/subscription.generated.ts`

**Interfaces:**
- Extends `RefreshDto` with `last_success_at: Option<OffsetDateTime>`.
- Extends public `Refresh` with `pendingState`, `lastSuccessAt`, and `entryIssues`.
- Produces `RefreshEntryIssue { code: "DUPLICATE_ENTRY", count }` only for degraded duplicate drops.

- [x] Write RED repository/API assertions for queued and running refinement, previous `last_success_at` after a failure, degraded duplicate issue counts, and empty issue arrays for non-degraded states.
- [x] Update subscription list/detail SQL to select `f.last_success_at AS refresh_last_success_at` and exact-run SQL to join `feeds` by `feed_id`.
- [x] Decode `last_success_at` into `RefreshDto` and update every literal fixture explicitly.
- [x] Add the public response types and pure mapping:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshEntryIssueResponse {
    code: &'static str,
    count: i32,
}

fn public_pending_state(refresh: &RefreshDto) -> Option<&'static str> {
    match (refresh.status, refresh.started_at, refresh.completed_at) {
        (RefreshStatus::Queued, None, None) => Some("QUEUED"),
        (RefreshStatus::Running, Some(_), None) => Some("RUNNING"),
        _ => None,
    }
}
```

- [x] Extend OpenAPI with `RefreshPendingState`, `RefreshEntryIssue`, date-time bounds, `maxItems: 8`, and strict required fields.
- [x] Regenerate TypeScript with `cd web && npm run generate:reader-types`, then run the drift gate.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test feed_subscription_contracts refresh -- --nocapture
cargo test --locked --test subscription_api refresh -- --nocapture
cargo test --locked --test openapi_contract
cd web
npm run generate:reader-types
npm run check:reader-types
npm run typecheck
cd ..
git diff --check
```

- [x] Commit and push: `feat: expose refresh observability`.

### Task 2: Focused frontend presentation model

**Files:**
- Create: `web/src/features/reader/refresh/refreshPresentation.ts`
- Create: `web/src/features/reader/refresh/refreshPresentation.test.ts`
- Modify: `web/src/features/reader/categories/CategoryList.tsx`
- Modify: `web/src/features/reader/categories/CategoryList.test.tsx`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: `web/src/shared/i18n/i18n.test.ts`

**Interfaces:**
- Produces `refreshPresentation(refresh)` and `formatRefreshTimestamp(value, locale)`.
- Produces distinct source-dot labels for queued, running, degraded, cooldown, error, and ready.

- [x] Write RED tests for all presentation kinds, invalid/null timestamps, duplicate issue counts, retry timing, and last-success fallback inputs.
- [x] Implement the pure model using the generated `Refresh` type:

```ts
export function refreshPresentation(refresh: Refresh | null): RefreshPresentation {
  if (!refresh) return { kind: "idle", isPending: false }
  if (refresh.pendingState === "QUEUED") return { kind: "queued", isPending: true }
  if (refresh.pendingState === "RUNNING") return { kind: "running", isPending: true }
  if (refresh.state === "DEGRADED") return { kind: "degraded", isPending: false }
  if (refresh.state === "BACKING_OFF") return { kind: "cooldown", isPending: false }
  if (refresh.state === "ERROR") return { kind: "error", isPending: false }
  return { kind: "ready", isPending: false }
}
```

- [x] Replace the local `CategoryList` switch with the shared presentation mapping and distinct accessible labels.
- [x] Add concise English/Chinese copy for queue, running, cooldown, partial duplicate count, retry time, and last success.
- [x] Verify:

```bash
cd web
npm run test -- --run src/features/reader/refresh/refreshPresentation.test.ts src/features/reader/categories/CategoryList.test.tsx
npm run typecheck
cd ..
git diff --check
```

- [x] Commit and push: `feat: distinguish refresh activity`.

### Task 3: ASTRYX selected-Feed summary and responsive behavior

**Files:**
- Create: `web/src/features/reader/refresh/RefreshStatusSummary.tsx`
- Create: `web/src/features/reader/refresh/RefreshStatusSummary.test.tsx`
- Modify: `web/src/features/reader/components/SourceTree.tsx`
- Create: `web/src/features/reader/components/SourceTree.test.tsx`
- Modify: `web/src/features/reader/components/ReaderToolbar.tsx`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/e2e/support/readerApiFixture.ts`
- Modify: `web/e2e/support/readerOrganizationFixture.ts`
- Modify: `web/e2e/reader-workspace.spec.ts`

**Interfaces:**
- Uses ASTRYX `StatusDot`, `Banner`, `Stack`, and `Text`.
- Adds `refresh.isDisabled` to the existing source-toolbar action contract.
- Renders one selected-Feed summary in both persistent and drawer source panes.

- [x] Write RED component tests for queued/running rows, disabled refresh action, ready last-success copy, degraded duplicate feedback, cooldown retry copy, error plus previous success, and locale switching.
- [x] Implement `RefreshStatusSummary` as a cardless section. Use `Banner` only for degraded, cooldown, and error; keep queued/running/ready as compact status rows.
- [x] Pass the selected Subscription refresh to the summary and disable the refresh button while pending.
- [x] Add only layout CSS: flush divider, wrapping, tabular timestamps, and compact spacing. Reuse existing tokens and reduced-motion rules.
- [x] Extend stateful Playwright fixtures and cover queued to running to ready, degraded duplicates, cooldown, compact source drawer, 44px actions, and zero horizontal overflow at all four viewports.
- [x] Verify:

```bash
cd web
npm run test -- --run src/features/reader/refresh/RefreshStatusSummary.test.tsx src/features/reader/ReaderWorkspace.test.tsx
npm run typecheck
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --grep "Reader refresh observability"
cd ..
git diff --check
```

- [x] Commit and push: `feat: show refresh health in reader`.

### Task 4: Real feed, full gates, documentation, and CI

**Files:**
- Modify: `tasks/todo.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/refresh-observability-v1-report.md`

**Interfaces:**
- Produces final local, real-feed, CI, commit, and push evidence.

- [ ] Run the production application and refresh `https://www.ithome.com/rss/` through local `agent-browser`. Verify fetched entries remain readable, a successful terminal status renders, last success is present, stored reload remains separate, console errors are empty, and mobile overflow is zero.
- [ ] Run the full fresh gates:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cd web
npm run check:reader-types
npm run typecheck
npm run test:ci
npm run build
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:e2e
cd ..
git diff --check
```

- [ ] Update `tasks/todo.md` only for queued/running and entry-level partial feedback. Keep AI, plugin, MCP, OIDC, OPML, sorting/cursor, admin, and release smoke open.
- [ ] Update `tasks/plan.md` to point at this detailed plan and record exact evidence in the report.
- [ ] Commit and push: `test: verify refresh observability`.
- [ ] Monitor the triggered CI only for concrete failures. Apply bounded fixes, then append final run evidence and push a `[skip ci]` report closeout.

## Plan self-review

- Spec coverage: queued/running refinement, cooldown, last success, partial duplicate issue count, API generation, ASTRYX presentation, locales, mobile, E2E, real feed, CI, and push each map to one task.
- Dependency order: authoritative Rust/OpenAPI projection, then pure Web presentation, then visual composition and browser verification, then full delivery gates.
- DDIA consistency: no redundant issue store is created; database-clock facts remain authoritative; public issues are deterministic and bounded.
- API consistency: existing state values remain stable, new fields are additive, strict, typed, generated, and redacted.
- UI consistency: refresh remains secondary to reading, source context owns status, and high-frequency reading actions gain no animation.
- Placeholder scan: no TBD, unbounded review, unspecified error behavior, or subagent step remains.
- Scope exclusions: refresh history, malformed-entry skipping, push transport, AI/plugin lifecycle issues, and admin observability remain separate slices.
