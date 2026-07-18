# User Preferences v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- One user-owned `user_preferences` row with portable SQLite, PostgreSQL, and MySQL constraints for locale, theme, Reader density, and reading scale.
- User-locked partial-update repository that preserves concurrent disjoint patches and redacts corrupt storage.
- Authenticated `GET/PATCH /api/v1/preferences` with strict JSON, CSRF, a separate per-user mutation limiter, no-store responses, and committed OpenAPI contracts.
- Generated TypeScript wire DTOs, a strictly validated presentation-only local hint, and a first-paint bootstrap that sets only approved presentation attributes.
- ASTRYX `MoreMenu`, `Dialog`, `SegmentedControl`, `RadioList`, `Button`, `Stack`, and `Banner` settings workflow.
- Non-blocking preference loading, optimistic save with rollback, draft preservation, logout cleanup, locale activation, ASTRYX list density, and article-only typography scaling.
- Deterministic production E2E coverage at 1280×800, 900×800, 390×844, and 360×800.

## Commits

- `8ada342 feat: add user preference storage`
- `ffb2570 feat: add user preference repository`
- `791de53 feat: expose user preferences api`
- `27e6932 feat: add preference runtime`
- `1d47e20 feat: add reader preference settings`

## Deterministic verification

- `npm run check:reader-types`: generated Reader and preference contracts current.
- `npm run typecheck`: passed.
- `npm run test:ci`: 37 files passed, 202 tests passed.
- `npm run build`: passed; production JavaScript is 559.92 kB before gzip and retains the existing Vite advisory above 500 kB.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --project reader-1280x800 --project reader-900x800 --project reader-390x844 --project reader-360x800`: 4 tests passed.
- `cargo test --locked --all-features`: 422 tests passed; 1 opt-in live RSS smoke was ignored because `RAINDROP_LIVE_RSS_SMOKE=1` was not set.
- `git diff --check`: passed.

The browser fixture keeps mutable server preference state across reloads, captures CSRF on PATCH, and can fail exactly one PATCH. It verifies a four-field partial mutation, persistence after reload, MoreMenu focus restoration, localized Dialog copy, mobile containment, theme/density/scale attributes, and failure rollback that preserves the unsaved draft before a successful retry.

## Real production browser evidence

Local `agent-browser` verification used the release binary, a temporary SQLite database, and the real setup/session/preferences API without request interception:

- Saved and observed `DARK + zh-CN + COMPACT + 120%`.
- Saved and observed `LIGHT + en + BALANCED + 110%`.
- Saved and observed `SYSTEM + en + SPACIOUS + 100%`; system mode removed the explicit `data-theme` attribute.
- Reload preserved the values. Clearing localStorage before reload still restored the persisted server values and recreated the validated presentation hint.
- At 390×844 the Settings Dialog was 351×685 at `(19.5, 79.5)` and fully contained.
- At 360×800 the Settings Dialog was 324×685 at `(18, 57.5)` and fully contained.
- Both mobile viewports had no document/body horizontal overflow, no Dialog overflow, and empty browser console/page error collections.
- Canceling mobile Settings reopened Sources and restored focus to the MoreMenu trigger.

The browser session, cookies, screenshots, setup token, and temporary SQLite directory were deleted after verification.

## Existing advisories

- Vite reports the existing production JavaScript chunk above 500 kB.
- Rust reports existing dead-code warnings in `validate_counts` and test support helpers.
- `proc-macro-error2 v2.0.1` remains a recorded future-incompatibility dependency advisory.

## Explicitly remaining

Sorting and reading cursor work remain unchecked even though the nested user-settings slice is complete. Registration/admin management, OIDC, AI providers and content artifacts, plugin lifecycle/SDK/sandbox, MCP client/server support, OPML import/export, portability CI, and release packaging also remain unchecked backlog work.
