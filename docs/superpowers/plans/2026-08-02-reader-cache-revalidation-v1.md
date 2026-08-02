# Reader Cache + Revalidation v1 Implementation Plan

**Status:** Implemented; release verification and delivery are recorded in the repository history and CI.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the authenticated Reader source tree and last queue from a bounded user-scoped browser cache, reconcile them in the background, reuse favicons across page reopen, and ship `v0.4.8`.

**Architecture:** A native IndexedDB adapter sits behind an injected `ReaderCache` interface. The cache stores only generated list DTOs and organization projections; the Reader reducer hydrates that normalized subset before existing API requests run, while request reducers preserve ready cached content until authoritative responses arrive. The existing server database and Feed `ETag`/`Last-Modified` pipeline remain unchanged.

**Tech Stack:** React 19, TypeScript 7, IndexedDB, Vitest 4, Rust 2024, Axum 0.8.

## Global Constraints

- No new Rust or npm dependency, database migration, service worker, public API schema, or full-article cache.
- Cache one active owner, all categories/subscriptions, and at most the last source's first 100 entry list DTOs for seven days.
- Never cache cookie/session/CSRF material, credentials, article HTML, inert-image metadata, profile data, preferences, or AI artifacts.
- Cache/storage failures degrade to the current server-first Reader and never block reading.
- Cached content remains visible during revalidation; this high-frequency path gets no entrance/list animation.
- Explicit logout, authenticated-session expiry, and owner change clear the active snapshot.
- Release fields and assertions end at exact version `0.4.8`; only intended files are committed.

---

## File map

- Create `web/src/features/reader/cache/readerCache.ts`: cache types, envelope validation, snapshot projection, IndexedDB storage, load/save/clear implementation.
- Create `web/src/features/reader/cache/readerCache.test.ts`: storage-boundary and red/green cache contract tests.
- Create `web/src/features/reader/model/reducerCache.ts`: normalized cache hydration only.
- Modify `web/src/features/reader/model/{types,reducer,reducerEntries,reducerSubscriptions,useReaderController,useReaderRequests}.ts`: initial route source, hydration action, cached-content-preserving revalidation, cache lifecycle.
- Modify `web/src/features/reader/model/useReaderController.test.tsx`: controller cache-first and authoritative-reconciliation tracer test.
- Modify `web/src/features/reader/ReadyPage.tsx` and `ReadyPage.test.tsx`: user/route wiring plus logout/401 clearing behavior.
- Modify `src/api/subscriptions.rs` and `tests/subscription_api.rs`: favicon HTTP cache contract.
- Modify version/release files: `README.md`, `Cargo.toml`, `Cargo.lock`, `web/package.json`, `web/package-lock.json`, `web/e2e/admin-only-setup.spec.ts`, `web/src/features/preferences/components/PreferencesDialog.test.tsx`.

### Task 1: Reader cache boundary

**Files:**
- Create: `web/src/features/reader/cache/readerCache.ts`
- Create: `web/src/features/reader/cache/readerCache.test.ts`

**Interfaces:**
- Produces: `ReaderCache`, `ReaderCacheSnapshot`, `ReaderCacheStorage`, `createReaderCache(storage, now)`, `browserReaderCache`, and `readerCacheSnapshot(state)`.
- Consumes: generated `Category`, `Subscription`, `EntryListItemResponse` validators and `ReaderSource`/`ReaderState`.

- [ ] **Step 1: Write a failing valid-snapshot test**

  Use an in-memory `ReaderCacheStorage` and assert `save("user-a", snapshot)` followed by `load("user-a")` returns the projection, while the raw envelope contains `schemaVersion: 1`, `ownerUserId`, and no string `csrf-memory`.

- [ ] **Step 2: Run the cache test red**

  Run: `npm --prefix web run test:ci -- readerCache.test.ts`

  Expected: FAIL because `readerCache.ts` and the exported cache factory do not exist.

- [ ] **Step 3: Implement the minimum cache factory and envelope**

  Implement this boundary:

  ```ts
  export interface ReaderCache {
    load(userId: string): Promise<ReaderCacheSnapshot | null>
    save(userId: string, snapshot: ReaderCacheSnapshot): Promise<void>
    clear(): Promise<void>
  }

  export function createReaderCache(
    storage: ReaderCacheStorage,
    now: () => number = Date.now,
  ): ReaderCache
  ```

  `load` validates exact keys, schema version, owner, age, array bounds, source shape, generated DTO guards, queue IDs, generation, scroll offsets, and a 2 MiB serialized ceiling. Any non-null invalid record is cleared and returned as a miss. All storage errors are caught as misses/no-ops.

- [ ] **Step 4: Add abuse-case tests one at a time**

  Add red/green cases for wrong owner, age over seven days, future schema, malformed DTO, more than 100 queue entries, queue/entity mismatch, cyclic/oversized raw input, read/write/clear rejection, and owner replacement. Each invalid load must clear once.

- [ ] **Step 5: Add the native IndexedDB adapter and projection builder**

  Store one record under a fixed key in database `raindrop-reader-cache`, object store `snapshots`, version `1`. Resolve unavailable/blocked/error/aborted IndexedDB operations as cache misses. `readerCacheSnapshot(state)` preserves ordered categories/subscriptions, current source queue, non-negative generation, bounded scroll anchors, and list DTOs with summaries capped at 512 Unicode scalar values.

- [ ] **Step 6: Run task verification**

  Run: `npm --prefix web run test:ci -- readerCache.test.ts && npm --prefix web run typecheck`

  Expected: both commands exit `0` with all cache cases executed.

### Task 2: Reducer hydration and non-destructive revalidation

**Files:**
- Create: `web/src/features/reader/model/reducerCache.ts`
- Modify: `web/src/features/reader/model/types.ts`
- Modify: `web/src/features/reader/model/reducer.ts`
- Modify: `web/src/features/reader/model/reducerSubscriptions.ts`
- Modify: `web/src/features/reader/model/reducerEntries.ts`
- Test: `web/src/features/reader/model/reducer.requests.test.ts`

**Interfaces:**
- Consumes: `ReaderCacheSnapshot` from Task 1.
- Produces: action `{ type: "readerCacheHydrated"; cached: ReaderCacheSnapshot }` and `initialReaderStateForSource(source)`.

- [ ] **Step 1: Write failing reducer tests**

  Assert a same-source cache makes subscriptions and queue ready with normalized entities; a different-source cache hydrates organization but not its queue; `subscriptionsRequested`/`sourceRequested` retain ready content; request failure retains cached content; a later received response authoritatively replaces additions, removals, order, counts, and state.

- [ ] **Step 2: Run reducer tests red**

  Run: `npm --prefix web run test:ci -- reducer.requests.test.ts`

  Expected: FAIL on the missing hydration action and current loading/error transitions.

- [ ] **Step 3: Implement cache hydration**

  `hydrateReaderCacheState` builds the existing normalized maps and orders, restores bounded scroll anchors, and only installs queue/generation when `sourceKey(cached.source) === sourceKey(state.selectedSource)`.

- [ ] **Step 4: Preserve cached rows during requests**

  A subscription request keeps status `ready` when organization data was hydrated. A source request/failure keeps status `ready` when that source key already has a queue property (including a valid cached empty queue). Cold requests retain existing `loading` and `error` semantics.

- [ ] **Step 5: Run task verification**

  Run: `npm --prefix web run test:ci -- reducer.requests.test.ts reducer.test.ts && npm --prefix web run typecheck`

  Expected: all selected tests and typecheck pass.

### Task 3: Controller cache-first lifecycle

**Files:**
- Modify: `web/src/features/reader/model/useReaderController.ts`
- Modify: `web/src/features/reader/model/useReaderRequests.ts`
- Modify: `web/src/features/reader/model/useReaderController.test.tsx`
- Modify: `web/src/features/reader/model/useReaderController.session.test.tsx`

**Interfaces:**
- Extends `UseReaderControllerOptions` with optional `userId`, `initialSource`, and injected `cache`.
- Extends `ReaderController` with `clearCache(): Promise<void>`.

- [ ] **Step 1: Write the failing cache-first tracer test**

  Inject a cache returning an old subscription/entry and delayed API promises. Start `load()` without awaiting it; assert cached state becomes `ready` before either promise resolves and remains present while requests are active. Resolve authoritative pages containing a new subscription/entry and omitting the old ones; assert the old projections are removed and the new projections win.

- [ ] **Step 2: Run the controller test red**

  Run: `npm --prefix web run test:ci -- useReaderController.test.tsx`

  Expected: FAIL because controller options and cache hydration do not exist.

- [ ] **Step 3: Implement hydrate-then-revalidate load**

  Deduplicate the cache-load promise, dispatch at most one hydration per mounted controller, then run the existing subscription/current-source requests. Initialize the reducer from `initialSource` so a deep-linked source only accepts its own queue cache.

- [ ] **Step 4: Implement serialized best-effort writes**

  After hydration/miss completes, debounce state projection writes briefly and chain them so completion order matches state order. Do not save blank pre-hydration state. Cancel timers on unmount and tolerate save failures.

- [ ] **Step 5: Implement expiry clearing**

  Wrap the current-session unauthenticated callback so `cache.clear()` settles before navigation to login. Expose `clearCache()` for explicit logout. Add session tests proving stale superseded `401` still cannot clear/expire the current session, while the current request's `401` clears exactly once.

- [ ] **Step 6: Run task verification**

  Run: `npm --prefix web run test:ci -- useReaderController.test.tsx useReaderController.session.test.tsx && npm --prefix web run typecheck`

  Expected: all selected tests and typecheck pass.

### Task 4: ReadyPage route/user/logout wiring

**Files:**
- Modify: `web/src/features/reader/ReadyPage.tsx`
- Modify: `web/src/features/reader/ReadyPage.test.tsx`

**Interfaces:**
- Consumes: `parseReaderPath`, `session.user.id`, and `controller.clearCache()`.
- Produces: cache scoped to the actual session user and initial Reader route.

- [ ] **Step 1: Write failing lifecycle tests**

  Add a delayed-network test that preloads the injected/browser cache for the session user and proves source/queue content appears before the response. Add logout verification that the cache clear completes before `onLoggedOut` and serialized cache data does not contain `csrfToken`.

- [ ] **Step 2: Run lifecycle tests red**

  Run: `npm --prefix web run test:ci -- ReadyPage.test.tsx`

  Expected: FAIL because ReadyPage does not pass user/source or clear cache.

- [ ] **Step 3: Wire route and user identity**

  Derive the first `ReaderSource` from the current canonical route, pass it and `session.user.id` to `useReaderController`, and await `controller.clearCache()` after successful server logout and before `onLoggedOut`.

- [ ] **Step 4: Run task verification**

  Run: `npm --prefix web run test:ci -- ReadyPage.test.tsx ReaderWorkspaceStates.test.tsx && npm --prefix web run typecheck`

  Expected: lifecycle and state tests pass with no new visual transition.

### Task 5: Favicon stale-while-revalidate policy

**Files:**
- Modify: `src/api/subscriptions.rs`
- Modify: `tests/subscription_api.rs`

**Interfaces:**
- Produces successful response header `Cache-Control: private, max-age=86400, stale-while-revalidate=604800` while retaining `Vary: Cookie` and failed-response `no-store`.

- [ ] **Step 1: Write a failing authenticated HTTP test**

  Add a static PNG `FeedTransport`, give the owned feed an HTTPS `site_url`, request its favicon, and assert `200`, `image/png`, exact cache policy, and `Vary: Cookie`. Keep the existing unsafe/missing-site test asserting `404` and `no-store`.

- [ ] **Step 2: Run the favicon test red**

  Run: `cargo test --locked --test subscription_api subscription_favicon -- --nocapture`

  Expected: the success test fails on the old `private, no-cache, max-age=0, must-revalidate` header.

- [ ] **Step 3: Change only the named favicon policy constant**

  Leave article media, sensitive JSON API responses, authentication, SSRF policy, body sniffing, and upstream transport behavior unchanged.

- [ ] **Step 4: Run task verification**

  Run: `cargo test --locked --test subscription_api subscription_favicon -- --nocapture && cargo fmt --all -- --check`

  Expected: favicon tests pass and formatting is clean.

### Task 6: UI/UX, security, and regression gates

**Files:**
- Modify only files exposed by failing checks; no opportunistic refactor.

**Interfaces:**
- Verifies all prior task contracts together.

- [ ] **Step 1: Run UI-focused tests and type/build gates**

  Run: `npm --prefix web run check:reader-types && npm --prefix web run typecheck && npm --prefix web run test:ci && npm --prefix web run build`

- [ ] **Step 2: Run Rust focused and full gates**

  Run: `cargo fmt --all -- --check && env -u RAINDROP_TEST_POSTGRES_URL -u RAINDROP_TEST_MYSQL_URL cargo test --locked --workspace --all-features`

- [ ] **Step 3: Run supply-chain audits required by the security skill**

  Run: `npm --prefix web audit --audit-level=high && npm --prefix web audit signatures && cargo audit`

  If `cargo audit` is unavailable, record that setup gap and rely on the repository CI audit job without claiming a local Cargo audit pass.

- [ ] **Step 4: Apply the named design checks**

  Confirm cache hits do not render skeletons, animate rows, change focus order, shift layout, or hide controls at wide/medium/390x844/360x800 layouts. Confirm cold/error states and reduced-motion behavior are unchanged.

### Task 7: Version `0.4.8`, review, commit, push, and tag

**Files:**
- Modify: `README.md`
- Modify: `Cargo.toml`
- Modify mechanically: `Cargo.lock`
- Modify: `web/package.json`
- Modify mechanically: `web/package-lock.json`
- Modify: `web/e2e/admin-only-setup.spec.ts`
- Modify: `web/src/features/preferences/components/PreferencesDialog.test.tsx`
- Include: feature spec and this plan.

**Interfaces:**
- Produces release commit on `main` and annotated `v0.4.8` tag.

- [ ] **Step 1: Update release notes and every version source**

  Add a concise Chinese `## v0.4.8` README entry describing cache-first Reader restoration, background reconciliation, and favicon reuse. Set exact version `0.4.8` in manifest/lockfiles and exact UI/E2E assertions.

- [ ] **Step 2: Run release contracts and release build**

  Run: `npm --prefix web run check:release-contracts && npm --prefix web run build && cargo build --release --locked && ./target/release/raindrop --version`

  Expected version output: `raindrop 0.4.8`.

- [ ] **Step 3: Run production E2E**

  Run: `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm --prefix web run test:e2e`

- [ ] **Step 4: Freeze and review the exact diff**

  Re-read `git status --short --branch -uall`, current `HEAD`, `git diff --check`, `git diff --stat`, full intended diff, dependency manifests, cache security boundaries, generated artifacts, and sibling cache/header patterns. Exclude `node_modules/.vite/**` from staging.

- [ ] **Step 5: Seed Release Gate 2.0**

  Run: `python3 /home/czyt/.cc-switch/skills/check/scripts/release_gate.py --root /home/czyt/code/rust/raindrop` and complete remaining artifact/CI rows from repository evidence.

- [ ] **Step 6: Commit intended files**

  Re-check that `HEAD` is still the recorded baseline, stage explicit paths only, scan the staged diff for secrets, and commit with `feat: cache reader state across reloads` (or split a final `release: v0.4.8` commit only if project version contracts require it).

- [ ] **Step 7: Push branch and annotated tag**

  Re-read status/HEAD, push `main`, create annotated tag `v0.4.8` at the pushed commit, push that exact tag, and verify `origin/main` plus `refs/tags/v0.4.8` resolve to the same SHA.

- [ ] **Step 8: Monitor tag workflows to terminal state**

  Discover runs for the tagged SHA with `gh run list --commit <sha> --json databaseId,name,status,conclusion,url`, poll each required CI/release/Docker run using `gh run view <id> --json status,conclusion,url`, and inspect the resulting GitHub release/assets when successful. Report an exact blocker instead of claiming release if any required run fails or remains non-terminal.

## Self-review

- Spec coverage: cache boundary, user isolation, bounded data, hydration, authoritative reconciliation, logout/401, favicon policy, UI stability, security audit, version/tag/CI all map to Tasks 1–7.
- Placeholder scan: no `TBD`, deferred behavior, unspecified test command, or unnamed interface remains.
- Type consistency: `ReaderCacheSnapshot`, `ReaderCache`, `readerCacheSnapshot`, `readerCacheHydrated`, `initialSource`, and `clearCache` names are consistent across producers and consumers.
