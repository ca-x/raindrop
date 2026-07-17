# Reader UI v1 — Task 4 report

## Task 4A

### Scope

Implemented the frontend-only keyboard, cursor, history, scroll, and focus slice. Playwright, `agent-browser`, live RSS, the Rust full suite, and `tasks/todo.md` are intentionally deferred to Task 4B.

### TDD evidence

- RED 1: `npx vitest run src/features/reader/keyboard/useReaderHotkeys.test.tsx` failed because `useReaderHotkeys` did not exist.
- GREEN 1: the focused hook suite passes with public ASTRYX 0.1.6 `useHotkeys`, pre-dispatch disabled state, editable-role/modal suppression, boundary behavior, repeat behavior, Shift behavior, and M/S repeat guards.
- RED 2: `ReaderKeyboardWorkspace.test.tsx` failed because N/P did not move an independent cursor, J/K did not navigate/replace history, M/S did not target the cursor, and no ASTRYX `Kbd` hints existed.
- GREEN 2: cursor selection/focus, J/K open-and-mark-read, N/P cursor-only movement, M/S cursor fallback, same-source replace history, compact Back focus, and localized visible hints pass.
- RED 3: `ReaderScrollWorkspace.test.tsx` failed because queue/article scroll containers did not restore or record route-keyed offsets.
- GREEN 3: queue and article offsets restore independently, clamp to their containers, record through `ReaderController.recordScrollAnchor`, and new article routes start at the top when no anchor exists.

### Verification

- Focused Reader tests: 7 files, 43 tests passed.
- `npm run check:reader-types`: generated Reader contracts are current.
- `npm run typecheck`: passed.
- `npm run test:ci`: 27 files, 124 tests passed.
- `npm run build`: passed; Vite emitted a new chunk-size advisory for the 504.19 kB main chunk, up from Task 3's 496.91 kB build.
- `git diff --check`: passed before the final commit gate.

### Changed files

- `web/src/features/reader/keyboard/useReaderHotkeys.ts`
- `web/src/features/reader/keyboard/useReaderHotkeys.test.tsx`
- `web/src/features/reader/ReaderKeyboardWorkspace.test.tsx`
- `web/src/features/reader/ReaderScrollWorkspace.test.tsx`
- `web/src/features/reader/routes/ReaderRoutes.tsx`
- `web/src/features/reader/layout/ReaderShell.tsx`
- `web/src/features/reader/components/EntryQueue.tsx`
- `web/src/features/reader/components/ArticleReader.tsx`
- `web/src/features/reader/components/ReaderToolbar.tsx`
- `web/src/features/reader/reader.css`
- `web/src/shared/i18n/i18n.ts`

### Self-review and concerns

- Queue business order is read only from `queueBySourceKey[sourceKey(selectedSource)]`; DOM lookup is used only to focus the already-selected entry ID.
- The queue cursor remains separate from the route-backed open article. N/P does not navigate, load detail, mutate state, or write history.
- Known Reader overlays provide controlled pre-keydown disabling; additional ARIA editable focus is tracked, and an immediate capture-phase modal guard stops Reader letters before ASTRYX can call `preventDefault`.
- Scroll operations use immediate `scrollTop` and `scrollIntoView({ behavior: "auto" })`; no animation or new runtime dependency was added.
- Every changed non-generated TS/TSX file remains at or below 250 lines.
- Task 4B still must provide deterministic four-viewport Playwright, production browser QA, and live IT Home RSS evidence.

### Fix Wave — Important review findings

- Source transition RED: switching the route from source A to B before the controller settled allowed A's cursor IDs and scroll container to remain active under B. GREEN: shortcuts, cursor reconciliation, focus, clicks, and queue anchor writes remain disabled until the route source matches `selectedSource` and the queue is ready.
- Pending merge RED: explicit merge preserved the article but left the old scroll/cursor position. GREEN: merge keeps the article and URL, records/restores queue offset `0`, and moves cursor/focus to the first merged entry.
- Article race RED: route B could temporarily bind the short article A node, clamp B's saved offset, and record the clamped value under B. GREEN: article scroll/focus effects bind only when the route entry, selected detail, and ready status all agree; old cleanup records only the old route.
- Modal RED: an immediately mounted native or ARIA modal could receive a same-task keydown before MutationObserver state disabled ASTRYX. GREEN: a feature-specific window capture guard stops unmodified Reader letters without `preventDefault`, while controlled overlay disabling remains in place.

Fix Wave verification: 3 focused files / 23 tests passed; Reader contracts and typecheck passed; the full frontend suite passed with 27 files / 129 tests; production build passed; `git diff --check` is part of the final commit gate.

Minor follow-up for final Reader review: the Vite main chunk warning is new relative to Task 3 (496.91 kB). Task 4A originally built at 504.19 kB and this Fix Wave builds at 504.83 kB; the threshold was not changed or hidden.

## Task 4B

### Scope and deterministic browser coverage

Added a production-binary Reader Playwright contract with stateful API fixtures and four dedicated projects. Bootstrap, setup, session, CSRF, embedded assets, routing, and the server process remain real; only Reader/Subscription API responses are deterministic fixtures.

| Project | Viewport | Proved behavior |
|---|---:|---|
| `reader-1280x800` | 1280×800 | Three panes, J/K/N/P/M/S, pressed state mutations, dialog shortcut suppression, browser Back, queue/article scroll restoration, pending merge focus/offset |
| `reader-900x800` | 900×800 | Queue/article layout, source MobileNav, source transitions, modal shortcut suppression |
| `reader-390x844` | 390×844 | Compact queue/detail routing, UI Back, browser Back/Forward, queue scroll and row focus restoration |
| `reader-360x800` | 360×800 | Direct deep link fallback Back, reduced motion, hostile-content containment |

Every project asserts document/body horizontal containment. The hostile detail includes a long token, 1600px inert image, wide table, long pre/code, iframe, and video. Publisher image URLs remain absent from rendered HTML.

### Browser-discovered product fixes

- Wide publisher tables kept `display: table`, so their minimum content width could exceed a 360px article. Reader CSS now gives `table` a block scroll container and explicitly keeps `pre` horizontally scrollable.
- Production Lingui catalogs were raw strings. Placeholders rendered literally and populated feeds flooded the console with `Uncompiled message detected`. Static messages now load in compiled form, placeholder messages are explicit compiled arrays, and raw braces fail fast.
- Accepted create/refresh operations returned PENDING but never reconciled their terminal subscription projection. The client now performs a one-second, 60-attempt bounded poll through the existing detail API, updates title/count/status, stops at the first non-PENDING state, expires on 401, and aborts on replacement, delete, unmount, or session expiry. An explicit aborted-signal gate prevents stale APIs from resurrecting deleted subscriptions.

### Local agent-browser and real IT之家 Feed

- `agent-browser doctor --offline --quick`: 8 pass, 0 fail; four unrelated stale-session warnings only.
- Completed real first-run setup in the release binary and submitted exactly `https://www.ithome.com/rss/` through `Add subscription`.
- Initial refresh persisted 60 unread entries; a later verification refresh increased the projection to 63.
- Stored reload emitted only `GET /api/v1/entries?...`; Feed refresh emitted `POST /api/v1/subscriptions/{id}/refresh`.
- Post-fix refresh emitted bounded detail polls and reached `IT之家 / Refresh complete / 63` without a page reload.
- Opened a real IT之家 article, exercised desktop browser Back and compact UI Back, and observed no horizontal overflow at 1280×800, 900×800, 390×844, or 360×800.
- Post-fix `agent-browser console` and `agent-browser errors` were empty.
- Sanitized screenshots, issue video, report, and plain command/result log are in `.superpowers/sdd/reader-ui-task-4-evidence/`. Tokens, credentials, cookies, browser state, and database files are not stored.

### Verification

- `npm run check:reader-types`: generated Reader contracts are current.
- `npm run typecheck`: passed.
- `npm run test:ci`: 28 files, 134 tests passed.
- `npm run build`: passed; main JS is 506.45 kB (158.29 kB gzip), with the acknowledged chunk advisory still visible.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:e2e`: release lib 102/102, embedded web 9/9, Playwright 14/14; all four Reader projects ran with no skips.
- `cargo test --locked --all-features`: passed. The opt-in public-network `live_rss_ithome` test remained the single expected ignored test; the agent-browser run supplied additive live evidence.
- `git diff --check`: passed before documentation finalization and is rerun at the commit gate.

### Changed files

- `web/e2e/reader-workspace.spec.ts`
- `web/e2e/support/readerApiFixture.ts`
- `web/e2e/support/readerAssertions.ts`
- `web/e2e/support/app.ts`
- `web/e2e/admin-only-setup.spec.ts`
- `web/e2e/mobile-foundation.spec.ts`
- `web/playwright.config.ts`
- `web/src/features/reader/reader.css`
- `web/src/features/reader/model/useSubscriptionActions.ts`
- `web/src/features/reader/model/useReaderController.mutations.test.tsx`
- `web/src/shared/i18n/i18n.ts`
- `web/src/shared/i18n/i18n.test.ts`
- `.superpowers/sdd/reader-ui-task-4-evidence/`
- `.superpowers/sdd/reader-ui-task-4-report.md`
- `tasks/todo.md`

### Bounded self-review and remaining concerns

- The main-Agent review found and closed one Important cancellation race: an API implementation that ignored abort could otherwise reinsert a deleted subscription.
- No runtime dependency, backend migration, test-only production route, second Reader controller/store, or public-network dependency was added to deterministic Playwright.
- The 506.45 kB main chunk advisory remains a Minor follow-up. The threshold was not raised.
- Task 4B does not claim categories, search, bulk mark-read, next-unread-source, AI, plugins, MCP, OPML, OIDC, theme completion, motion polish, or release completion.
