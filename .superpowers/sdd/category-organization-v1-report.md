# Category Organization v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- User-scoped one-level category schema, repository, CRUD API, OpenAPI artifact, and generated TypeScript client.
- Nullable subscription category assignment with cross-user redaction and `ON DELETE SET NULL` behavior.
- Category-filtered Reader queries with cursor/filter binding and feed/category mutual exclusion.
- ASTRYX category tree and management flow for create, rename, delete, assign, and clear assignment.
- Responsive category routes, history behavior, focus restoration, Dialog containment, and deterministic four-viewport browser coverage.
- Stateful E2E organization fixtures split into focused modules rather than growing the Reader spec into one large file.

## Deterministic verification

- `npm run check:reader-types`: generated Reader contracts current.
- `npm run typecheck`: passed.
- `npm run test:ci`: 32 files passed, 156 tests passed.
- `npm run build`: passed; production JavaScript chunk is 542.21 kB before gzip and retains the existing Vite advisory above 500 kB.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --project reader-1280x800 --project reader-900x800 --project reader-390x844 --project reader-360x800`: 4 tests passed.
- `cargo test --locked --all-features`: 406 tests passed, 1 opt-in live RSS smoke ignored because `RAINDROP_LIVE_RSS_SMOKE=1` was not set.
- `git diff --check`: passed.

The browser fixture verifies category POST/PATCH/DELETE mutations, subscription PATCH, CSRF propagation, category-filtered entry pages, delete-to-uncategorized behavior, and 404 responses for cross-user category and subscription IDs.

## Real IT Home RSS evidence

Local `agent-browser` verification used `https://www.ithome.com/rss/` against the production binary without API interception:

- Imported 60 entries from the real Feed.
- Created category `科技` and assigned the IT之家 subscription.
- Loaded 50 first-page rows from the category route and opened an article from a category deep link.
- Reload preserved the assignment, category filter, selected category, and selected article.
- Manual Feed refresh reached `Refresh complete`.
- At 390×844 the category Dialog stayed inside the viewport, the document had no horizontal overflow, and browser console/page errors were empty.

Temporary screenshots were reviewed locally and were not committed because they can be reproduced from the browser scenarios.

## Toolbar fit verification

The 240px source panel now uses the semantic Raindrop image without duplicate visible brand text. At 1280×800:

- Manage categories: x=88..132, 44×44.
- Add subscription: x=136..180, 44×44.
- Sign out: x=184..228, 44×44.
- Source divider: x=240, leaving 12px after the final hit target.
- Document and body horizontal overflow checks passed; console and page errors were empty.

## Design review results

- Motion opportunity review recommended no additional category animation. Category selection, counts, routing, and keyboard actions are high-frequency functional state, while ASTRYX already provides Dialog, AlertDialog, Selector, and drawer transitions.
- AI-slop review found no confirmed issue in the category UI. Setup/login eyebrow copy and article kicker text carry real context; the only emoji match was test data.

## Existing advisories

- Vite reports the existing production JavaScript chunk above 500 kB.
- Rust reports existing dead-code warnings in `validate_counts` and some test support helpers.
- `proc-macro-error2 v2.0.1` remains a recorded future-incompatibility dependency advisory.

## Explicitly remaining

User settings and ordering, registration/admin management, OIDC, OPML import/export, AI providers and content artifacts, plugin lifecycle/SDK/sandbox, MCP client/server support, portability CI, and release packaging remain unchecked backlog work.
