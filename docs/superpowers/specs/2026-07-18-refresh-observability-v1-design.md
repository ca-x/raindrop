# Refresh Observability v1 Design

## Objective

Deliver the remaining RSS core refresh-observability slice. Users must be able to distinguish queued work from an actively running network refresh, understand cooldown and retry timing, see the last successful refresh time, and receive bounded entry-level feedback when a refresh completes partially.

The Reader keeps stored-entry reload and network refresh as separate actions. Refresh feedback stays secondary to reading, uses the existing ASTRYX shell, and remains equivalent across wide, medium, 390x844, and 360x800 layouts.

## Assumptions and scope decisions

- The database remains the source of truth for refresh lifecycle timestamps and status.
- No new table or dependency is required. Existing `feed_refresh_runs` and `feeds.last_success_at` already contain the authoritative facts.
- Current partial completion is caused by duplicate entries in one fetched document. The public contract exposes an aggregate entry issue with code `DUPLICATE_ENTRY` and a bounded count, never raw GUIDs, URLs, titles, feed content, or internal errors.
- Existing public `state` values remain unchanged. A new `pendingState` refines only `PENDING`, avoiding a semantic rewrite of terminal states.
- The user delegated specification and review decisions to the main Agent. No subagent or additional confirmation gate is used.

## Public API contract

The existing `Refresh` object adds three fields:

```json
{
  "state": "PENDING",
  "pendingState": "RUNNING",
  "lastSuccessAt": "2026-07-18T07:30:00.000000Z",
  "entryIssues": []
}
```

`pendingState` is `QUEUED`, `RUNNING`, or `null`:

- `QUEUED`: `state` is `PENDING`, `startedAt` and `completedAt` are null.
- `RUNNING`: `state` is `PENDING`, `startedAt` is present, `completedAt` is null.
- `null`: the refresh is terminal.

`lastSuccessAt` is the database-clock `feeds.last_success_at` projection and may be null when a Feed has never completed successfully.

`entryIssues` is a bounded array of `RefreshEntryIssue` objects:

```json
{
  "code": "DUPLICATE_ENTRY",
  "count": 2
}
```

- Maximum array length is 8.
- `count` is a positive integer.
- For the current parser, `DEGRADED` returns exactly one `DUPLICATE_ENTRY` issue whose count equals `droppedCount`.
- Other states return an empty array.
- Internal parser, persistence, SQL, HTTP, and provider error strings remain redacted behind the existing public error codes.

The response stays strict, user scoped, `no-store`, and generated into TypeScript from committed OpenAPI. No new endpoint is introduced.

## Data and consistency design

`RefreshDto` adds `last_success_at: Option<OffsetDateTime>`. Subscription list/detail projections select `feeds.last_success_at`; exact refresh replay/load joins the run to its owning Feed.

The projection deliberately does not persist a second issue log:

- `feed_refresh_runs.dropped_count` is the authoritative aggregate.
- `PARTIAL` already means persistence succeeded while duplicate document entries were ignored.
- `entryIssues` is a deterministic public projection of those facts.
- Feed retention continues to own refresh-run history without a new cascade or cleanup path.

This avoids write amplification and consistency drift between run counts, issue rows, and lifecycle outbox events. If future processors introduce multiple entry issue types, a later additive schema can persist bounded reason counts without changing the v1 issue object.

## Reader presentation

### Source tree status

The existing ASTRYX `StatusDot` labels become distinct:

- Queued: warning, pulsing, “Queued for refresh”.
- Running: warning, pulsing, “Refreshing”.
- Ready: success, “Refresh complete”.
- Degraded: warning, “Completed with skipped entries”.
- Backing off: warning, “Cooling down”.
- Error: error, “Refresh failed”.

The refresh action is disabled while `pendingState` is `QUEUED` or `RUNNING`, preventing a predictable `REFRESH_IN_PROGRESS` conflict.

### Selected Feed summary

A focused `RefreshStatusSummary` appears below the source toolbar only when a Feed is selected:

- Queued/running/ready use a compact status row with ASTRYX `StatusDot`, `Stack`, and `Text`.
- Degraded/backing-off/error use an ASTRYX `Banner` with stable, concise copy.
- Partial completion says exactly how many duplicate entries were ignored.
- Cooldown shows the retry time when present.
- The last successful refresh time is shown when present, including after a later failure.
- Dates use `Intl.DateTimeFormat` with the active locale and no process-local relative timer.

The component is cardless and flush with the source panel. It adds no decorative animation. Existing pulsing status respects reduced motion.

### Responsive behavior

- Wide layouts show the summary in the persistent source pane.
- Medium and compact layouts show the same summary inside the existing sources drawer.
- Copy wraps naturally, keeps 44px actions, and never introduces horizontal scrolling.
- Chinese and English use the same information hierarchy and semantic labels.

## Project structure and code style

- `src/feeds/dto.rs`: internal refresh projection.
- `src/feeds/subscription.rs`: portable SQL projection and decoding.
- `src/api/subscriptions.rs`: stable public refresh mapping.
- `docs/openapi/subscription-v1.json`: sole wire-contract source.
- `web/src/features/reader/refresh/refreshPresentation.ts`: pure state and copy input derivation.
- `web/src/features/reader/refresh/RefreshStatusSummary.tsx`: selected Feed ASTRYX presentation.
- `web/src/features/reader/categories/CategoryList.tsx`: compact source-dot presentation only.
- `web/src/features/reader/components/SourceTree.tsx`: selected Feed composition and action availability.
- `web/src/features/reader/reader.css`: feature-scoped layout only.

Rust uses typed enums and explicit conversions. TypeScript uses generated wire DTOs plus a focused pure presentation model:

```ts
export type RefreshPresentationKind =
  | "idle"
  | "queued"
  | "running"
  | "ready"
  | "degraded"
  | "cooldown"
  | "error"
```

No component redefines wire types and no large toolbar/controller file absorbs refresh formatting.

## Commands

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cd web
npm run generate:reader-types
npm run check:reader-types
npm run typecheck
npm run test:ci
npm run build
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:e2e
```

Local browser verification uses the production application, local `agent-browser`, and `https://www.ithome.com/rss/`.

## Testing strategy

- Rust DTO/repository tests cover queued, running, ready, degraded, backing-off, error, nullable last success, and exact database timestamps.
- OpenAPI tests require the new fields, strict enums, bounds, and generated TypeScript drift gate.
- Frontend pure tests cover every presentation kind, issue count copy inputs, retry time, and invalid/missing timestamps.
- Component tests cover source-dot labels, disabled pending action, selected Feed summary, partial issue feedback, last success after failure, and both locales.
- Playwright covers queued to running to ready polling, degraded duplicate feedback, cooldown, compact source drawer, touch targets, and overflow.
- The real IT Home feed is refreshed again to verify successful production behavior and console/page error absence.

## Boundaries

- Always: keep database timestamps authoritative, generate TypeScript from OpenAPI, redact internal error detail, use ASTRYX components, verify mobile layouts, and run fresh gates before commit.
- Main-Agent decision: the additive response fields and query projection are authorized by the user's autonomous internal-review instruction.
- Never: add a second refresh history table, expose raw entry/feed identifiers in issue feedback, add client-side countdown polling, merge stored reload with network refresh, modify `.superpowers/research/`, modify root `node_modules/`, or use subagents.

## Success criteria

1. A queued refresh and a running refresh have distinct accessible labels while preserving `state: PENDING` compatibility.
2. A pending Feed cannot submit a redundant manual refresh from the Reader.
3. A partial refresh identifies the affected entry count and `DUPLICATE_ENTRY` reason without exposing raw feed content.
4. Cooldown shows retry timing and a later failure still shows the previous successful refresh time.
5. Subscription list, detail, refresh replay, OpenAPI, and generated TypeScript agree on the new fields.
6. Wide, medium, 390x844, and 360x800 layouts remain readable, touch safe, and free of horizontal overflow.
7. Rust, Web, Playwright, real IT Home browser verification, CI, commit, and push gates pass.

## Out of scope

- Full refresh history UI.
- Per-entry raw title, URL, GUID, or parser diagnostic disclosure.
- Skipping individually malformed entries. Current parser still fails the complete document for unsafe or structurally invalid content.
- SSE/WebSocket push, relative countdown timers, desktop notifications, or administrator refresh dashboards.
- AI/plugin lifecycle execution and plugin-generated entry issues.

## Open questions

None. The main Agent resolved the scope through the bounded DDIA, API, and UI review above.
