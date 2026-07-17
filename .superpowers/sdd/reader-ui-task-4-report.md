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
- `npm run build`: passed; Vite emitted its existing chunk-size advisory for the 504.19 kB main chunk.
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
- Known Reader overlays provide controlled pre-keydown disabling; additional ARIA editable focus and uncontrolled modal presence are tracked before ASTRYX can call `preventDefault`.
- Scroll operations use immediate `scrollTop` and `scrollIntoView({ behavior: "auto" })`; no animation or new runtime dependency was added.
- Every changed non-generated TS/TSX file remains at or below 250 lines.
- Task 4B still must provide deterministic four-viewport Playwright, production browser QA, and live IT Home RSS evidence.
