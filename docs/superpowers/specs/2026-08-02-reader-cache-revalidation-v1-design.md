# Reader Cache + Revalidation v1

**Status:** Implemented and verified for the `v0.4.8` release.

## Assumptions and confirmed facts

- Raindrop remains a self-hosted, authenticated web application backed by SQLite, PostgreSQL, or MySQL.
- Feed documents and entries are already durable server-side. Feed refresh already persists `ETag` and `Last-Modified`, sends conditional requests, handles `304 Not Modified`, and deduplicates entries by stable identity.
- The reported reload delay is therefore a browser Reader-state problem: React starts with an empty source tree and queue and waits for authenticated API reads. Subscription favicons are also proxied upstream again because their current response requires revalidation and has no reusable validator.
- “Incremental update” in this slice means cache-first Reader hydration followed by authoritative background reconciliation, while upstream RSS network refresh keeps using its existing HTTP conditional-request path. A new delta API is not required for the user-visible outcome.
- The next project-convention patch release is `v0.4.8`.

## Objective

Make reopening the Reader feel immediate without weakening authentication or showing one user's data to another user on the same browser.

For the authenticated user, Raindrop restores the subscription/category tree and the most recently viewed source's first queue page from a bounded browser cache. It then revalidates those projections against the server in the background. Cached content stays visible while revalidation is running or temporarily fails; a successful response remains authoritative.

Subscription favicons use a private stale-while-revalidate browser policy so routine page reopen does not block on refetching every visible site's icon.

## Tech stack

- Backend: Rust 2024, Axum 0.8, SeaORM 1.1; no database migration or new Rust dependency.
- Frontend: React 19, TypeScript 7, Vite 8, Vitest 4.
- Cache storage: native IndexedDB behind a small injected `ReaderCache` interface; no new npm dependency and no service worker.
- Wire DTO validation: reuse generated `isSubscription`, `isCategory`, and `isEntryListItemResponse` guards.

## Commands

- Frontend typecheck: `npm --prefix web run typecheck`
- Frontend tests: `npm --prefix web run test:ci`
- Frontend production build: `npm --prefix web run build`
- Generated contract check: `npm --prefix web run check:reader-types`
- Release contract check: `npm --prefix web run check:release-contracts`
- Rust format check: `cargo fmt --all -- --check`
- Rust tests: `env -u RAINDROP_TEST_POSTGRES_URL -u RAINDROP_TEST_MYSQL_URL cargo test --locked --workspace --all-features`
- Release build: `cargo build --release --locked`
- Production E2E: `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm --prefix web run test:e2e`

## Project structure

- `web/src/features/reader/cache/`: cache envelope validation, IndexedDB adapter, and cache tests.
- `web/src/features/reader/model/`: cache hydration/reconciliation in the existing normalized Reader controller and reducer.
- `web/src/features/reader/ReadyPage.tsx`: supplies the authenticated user identity and clears cached Reader data on explicit logout.
- `src/api/subscriptions.rs`: favicon cache policy only.
- `docs/superpowers/specs/`: this living feature contract.
- `README.md`, `Cargo.toml`, `Cargo.lock`, `web/package*.json`, and version assertions: `v0.4.8` release synchronization.

## Code style

Use the existing explicit runtime-validation and reducer-action style. Cache failures are optional infrastructure failures, not Reader failures.

```ts
const cached = await cache.load(userId, source)
if (cached && session.active()) {
  dispatch({ type: "readerCacheHydrated", cached })
}
await Promise.all([loadSubscriptions(), loadSource(source, "replace")])
```

- TypeScript uses named domain types, strict unknown-value validation, and no duplicate hand-written wire DTOs.
- Rust cache policy remains a named constant covered through the HTTP response seam.
- Do not add a generalized state-management or caching framework for this one projection.

## Cache contract

- One schema-versioned active snapshot is stored per origin, with an exact `ownerUserId`.
- A different authenticated user clears the prior active snapshot before any hydration; malformed, future-version, expired, oversized, or source-inconsistent snapshots are ignored and removed.
- Maximum age is seven days. Every cache hit still starts server reconciliation.
- The snapshot contains categories, subscriptions, the last selected `ReaderSource`, at most the API's first 100 queue entries, their generated list DTOs, the queue snapshot generation, and the saved scroll anchor for that source.
- Cached summaries are reduced to the same bounded preview used by the queue. The complete article body, inert-image source metadata, AI artifacts, profile/preferences, cookies, CSRF token, session expiry, credentials, and provider secrets are never cached here.
- Cache reads/writes/deletes are best effort. IndexedDB denial, quota exhaustion, unavailable APIs, or corrupt data must fall back to today's server-first behavior without a new error screen.
- Successful subscription, category, entry-state, source, and list mutations schedule a bounded cache rewrite. Writes are serialized so an older async write cannot overwrite newer state.
- Explicit logout and authenticated-session expiry clear the active Reader snapshot before returning to login.

## Revalidation and UI behavior

- A valid cache makes the source tree and matching queue `ready`; it must not be replaced by skeletons while background requests are active.
- The server response is authoritative: additions, removals, order, unread/star state, category placement, titles, counts, and snapshot generation reconcile into the normalized state.
- If background reconciliation fails after hydration, cached content remains operable and the existing manual stored-entry reload stays available. A cold load with no cache keeps the existing error behavior.
- Cache hydration and reconciliation add no entrance animation, shimmer over cached rows, list stagger, or scroll animation. This is a high-frequency path; visual stability and immediate interaction are the feedback.
- Existing focus order, deep links, keyboard navigation, reduced-motion behavior, mobile layouts, and 44 px touch targets remain unchanged.
- Favicon responses use `private, max-age=86400, stale-while-revalidate=604800` plus the existing `Vary: Cookie`; failures remain `no-store`.

### Emil design review

| Before | After | Why |
| --- | --- | --- |
| Reload starts with source/queue skeletons | Valid cached rows render as ready content | Preserve continuity and perceived speed without decorative motion |
| Revalidation can replace usable content with loading UI | Cached content stays stable while the request runs | Background work should not interrupt reading |
| Visible favicons re-proxy on routine reopen | Fresh icons reuse private cache; stale icons refresh in the background | Avoid repeated visual pop-in and upstream latency |
| Cache hit could invite a reveal animation | Cache hit is rendered without a new animation | This path is seen many times per day and should feel instantaneous |

## Testing strategy and confirmed seams

Tests exercise public behavior rather than IndexedDB implementation details:

1. **Reader cache seam** — `ReaderCache.load/save/clear`: accepts a valid same-user snapshot; rejects and clears corrupt, expired, oversized, wrong-owner, and wrong-source snapshots; storage errors degrade to a miss.
2. **Reader controller seam** — `useReaderController.load`: with a delayed API, cached subscriptions and matching queue become ready first and remain visible; resolving the API replaces them with authoritative additions/removals/state. With no cache, existing skeleton/error behavior remains.
3. **Session seam** — `ReadyPage`: explicit logout and a current-session `401` clear the active Reader cache, while cache values never contain the CSRF token.
4. **HTTP media seam** — authenticated favicon response: success carries the private stale-while-revalidate policy and `Vary: Cookie`; failure remains `no-store`.
5. **Production seam** — embedded release/E2E: reload the same authenticated Reader route and observe stable source/queue content, correct deep-link behavior, no page/console errors, and responsive layouts.

Each production change follows one red → green slice at these seams. Existing feed transport/persistence tests remain the regression proof for `ETag`, `Last-Modified`, `304`, and entry deduplication.

## Boundaries

- Always: validate cached unknown data; scope it to the authenticated user; cap age/count/text; preserve server authority; clear on logout/expiry/owner change; keep cache failure non-blocking; run release/version contracts before tagging.
- Ask first: change server API schemas, add a database migration, add a dependency, cache full article bodies, cache more than the last source page, or add offline mutations.
- Never: cache cookies, CSRF/session material, credentials, Provider keys, unsanitized feed HTML, AI artifacts, or data from multiple users in the active snapshot; weaken API `no-store` headers; make cache availability required for reading.

## Success criteria

1. On a repeated visit to the same Reader source, valid cached subscriptions and queue are available before delayed network responses and no skeleton replaces them during revalidation.
2. The background result authoritatively adds, removes, reorders, and updates entries/subscriptions without duplicate entities or stale unread counts.
3. Cold load, corrupt cache, disabled IndexedDB, quota/storage errors, and seven-day expiry behave like the current server-first Reader.
4. A different user, logout, or authenticated-session expiry cannot hydrate the previous user's snapshot.
5. Cache storage contains no CSRF token, cookie/session value, full article body, inert-image metadata, AI output, or credential.
6. Routine favicon reuse has the specified private stale-while-revalidate policy; failed media responses remain non-cacheable.
7. Existing Reader keyboard, focus, scrolling, responsive, localization, source refresh, and feed conditional-request tests remain green.
8. Version fields and release assertions are synchronized to `0.4.8`; the reviewed commit is pushed to `main`, annotated tag `v0.4.8` is pushed, and the tag-triggered GitHub workflows are checked to a terminal result.

## Open questions

- None. The user delegated implementation decisions, and the five test seams above were exercised.
