# OPML and CommaFeed Alignment v0.2.0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute inline with `executing-plans`; the user explicitly requested the main agent only. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Raindrop v0.2.0 with safe OPML import/export, a CommaFeed-familiar reader/settings flow, a responsive settings dialog, and no 12-character setup-password minimum.

**Architecture:** Add a bounded streaming OPML parser and a user-scoped import repository beside the existing feed repository. Preview is read-only; commit revalidates the document and writes categories/subscriptions in one database transaction, while feed refresh remains post-commit asynchronous. Expose the feature through the existing authenticated reader API and a modular ASTRYX transfer panel embedded in settings.

**Tech Stack:** Rust 2024, Axum 0.8, SeaORM 1.1, quick-xml 0.41, React 19, TypeScript 7, ASTRYX 0.1.6, Vitest, Playwright, agent-browser.

## Global Constraints

- Preserve unrelated uncommitted AI Reader/provider work and do not include it in v0.2.0 commits.
- OPML input is limited to 10 MiB and 10,000 outline elements; external entities/DTD are rejected.
- All imported and exported resources are scoped to the authenticated user.
- Import preview performs no writes; commit is atomic for database writes and duplicate-safe.
- Network feed refresh never runs inside the import transaction.
- UI copy is available in `zh-CN` and `en`; use ASTRYX components before custom controls.
- Frequent reader/keyboard interactions do not animate; occasional dialogs use explicit sub-300ms ease-out transitions and honor reduced motion.
- Verify the live subscription path with `https://www.ithome.com/rss/`.

---

### Task 1: Remove the setup password minimum

**Files:**
- Modify: `src/auth/users.rs`
- Modify: `web/src/features/setup/model.ts`
- Modify: `web/src/features/setup/model.test.ts`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: setup/auth Rust integration tests containing minimum-length expectations

- [ ] Replace the 12-byte rule with a non-empty password rule at both server and client boundaries.
- [ ] Update Chinese and English guidance/error copy.
- [ ] Add tests proving a short non-empty password succeeds and an empty password is rejected.

### Task 2: Add bounded OPML parsing, preview, transactional import, and export

**Files:**
- Create: `src/feeds/opml.rs`
- Modify: `src/feeds/mod.rs`
- Create: `src/api/subscriptions/opml.rs`
- Modify: `src/api/subscriptions.rs`
- Create: `tests/opml_api.rs`

**Interfaces:**
- `OpmlDocument::parse(&[u8]) -> Result<OpmlDocument, OpmlError>` returns normalized feed entries plus invalid counts without resolving or fetching URLs.
- `FeedRepository::preview_opml(user_id, document) -> Result<OpmlPreview, OpmlError>` reports valid, duplicate, invalid, and new counts.
- `FeedRepository::import_opml(user_id, document) -> Result<OpmlImportResult, OpmlError>` commits categories and subscriptions atomically.
- `FeedRepository::export_opml(user_id) -> Result<Vec<u8>, OpmlError>` returns deterministic UTF-8 XML with escaped titles and category nesting.
- `POST /api/v1/imports/opml?mode=preview|commit` consumes raw OPML and returns one stable JSON result shape.
- `GET /api/v1/exports/opml` returns `application/xml; charset=utf-8` and an attachment filename.

- [ ] Write parser tests for nested categories, flat feeds, duplicate URLs, invalid URLs, DTD/entity rejection, outline and byte limits, and XML escaping.
- [ ] Write API tests for authentication, CSRF on commit, preview no-write behavior, duplicate-safe commit, user isolation, rollback, and export round-trip.
- [ ] Implement the minimal parser, repository transaction, API mapping, and post-commit feed-runtime notification.
- [ ] Run focused Rust tests and formatting/lints.

### Task 3: Add the ASTRYX OPML transfer UI and repair settings layout

**Files:**
- Create: `web/src/features/opml/api.ts`
- Create: `web/src/features/opml/model.ts`
- Create: `web/src/features/opml/components/OpmlTransferPanel.tsx`
- Create: `web/src/features/opml/components/OpmlTransferPanel.test.tsx`
- Modify: `web/src/features/preferences/components/PreferencesDialog.tsx`
- Modify: `web/src/features/preferences/components/PreferencesDialog.test.tsx`
- Modify: `web/src/features/reader/layout/ReaderShell.tsx`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/src/shared/i18n/i18n.ts`

- [ ] Use ASTRYX `TabList`, `FileInput`, `Banner`, `Button`, `Layout`, and `Stack` for separate Appearance and Subscriptions panels.
- [ ] Preview immediately after file selection, show valid/new/duplicate/invalid counts, and require an explicit import action.
- [ ] Export through an authenticated blob download so API errors remain visible.
- [ ] Make the dialog header/footer stable and the content region independently scrollable at desktop, short viewport, and mobile sizes.
- [ ] Keep actions reachable with safe-area padding and restore focus to the settings trigger.

### Task 4: Align the high-frequency reader flow with CommaFeed

**Files:**
- Modify: `web/src/features/reader/components/SourceTree.tsx`
- Modify: `web/src/features/reader/categories/CategoryList.tsx`
- Modify: `web/src/features/reader/components/EntryQueue.tsx`
- Modify: `web/src/features/reader/components/ArticleReader.tsx`
- Modify: `web/src/features/reader/reader.css`
- Add or modify focused component tests beside each component

- [ ] Improve source hierarchy and unread-count alignment, keep smart views first, and reduce secondary URL noise in the tree.
- [ ] Make queue rows scan like CommaFeed: clearer unread weight, stable date column, feed/author metadata before summary, and stronger selected-row contrast.
- [ ] Keep the article toolbar visible while the article scrolls and tighten article metadata/title rhythm without shrinking tap targets.
- [ ] Add press feedback only to pointer-activated controls; do not animate keyboard navigation or repeated list selection.
- [ ] Verify no horizontal overflow at 320px and usable three-pane density at 1280px.

### Task 5: Verify and publish v0.2.0

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `web/package.json`
- Modify: `web/package-lock.json`
- Modify: `README.md` and release-facing notes if present

- [ ] Run Rust unit/integration tests, frontend typecheck/Vitest/build, and focused Playwright coverage.
- [ ] Start the app, subscribe to `https://www.ithome.com/rss/`, import/export an OPML file, and validate desktop/mobile UI using local `agent-browser`.
- [ ] Inspect the exact staged diff and secret scan; commit only v0.2.0 files and push `main`.
- [ ] Verify the committed revision in a clean worktree, create annotated tag `v0.2.0`, and push the tag.

## Emil UI Review

| Before | After | Why |
| --- | --- | --- |
| One tall settings form with a visually detached radio list | Two focused tabs: Appearance and Subscriptions, with consistent field grouping | Progressive disclosure reduces scanning and gives OPML a predictable home like CommaFeed |
| Dialog content competes with the footer and can exceed short viewports | Fixed header/footer with one independently scrollable content region | Primary actions remain reachable without hiding settings content |
| Source rows mix title, site URL, status, and unread count at equal visual weight | Feed title and unread count form the scan line; status is secondary and URLs leave the navigation tree | The sidebar becomes faster to parse and closer to a dedicated feed navigator |
| Queue title, metadata, summary, and date have weak alignment | Unread weight, stable metadata order, clipped summary, and fixed date column | Repeated rows become comparable at a glance |
| Article actions scroll away with content | Sticky article toolbar with restrained surface separation | Frequent read/star/original actions remain available without extra scrolling |
| Generic or broad transitions risk slowing repeated navigation | Explicit transform/color transitions under 180ms only for pointer press/occasional overlays | Feedback feels immediate while keyboard-heavy reading stays instant |

## Self-review

- Spec coverage: OPML limits, preview, user transaction, duplicate behavior, export metadata, responsive UI, bilingual copy, password rule, live RSS verification, and v0.2.0 publication are all assigned.
- Placeholder scan: no implementation placeholders or deferred decisions remain in this plan.
- Type consistency: backend preview/import result names are mirrored by the frontend model and one API response shape.
