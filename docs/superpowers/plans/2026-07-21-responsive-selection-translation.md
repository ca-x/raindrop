# Responsive Selection Translation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep reader navigation responsive while translation is pending and add a safe, accessible floating popover for looking up or translating selected article text.

**Architecture:** Long-running translation promises remain in the translation controller and are started from ordinary event handlers, never React transitions. A new authenticated `/translate` endpoint translates one bounded text selection with the user's saved provider configuration; right-clicking an eligible selection snapshots text wholly inside the current article, opens a cursor-anchored popover, automatically starts lookup or paragraph translation, and renders results as escaped React text.

**Tech Stack:** Rust/Axum, React 19, React Router 7, ASTRYX Popover, TypeScript, Vitest, OpenAPI-generated reader contracts.

## Global Constraints

- Do not run a local Docker build; verify Docker only with GitHub Actions.
- Keep word lookup limited to 200 Unicode scalar values.
- Limit selected paragraph translation to 8,000 Unicode scalar values and reject disallowed control characters on the server.
- Treat selected article text and provider output as untrusted data; never insert either with `innerHTML`.
- Do not move the already-published `v0.3.5` tag; release this work as `v0.3.6`.

---

### Task 1: Navigation regression guard

**Files:**
- Create: `web/src/features/translation/reader/TranslationReaderControls.test.tsx`
- Modify: `web/src/features/translation/reader/TranslationReaderControls.tsx`

**Interfaces:**
- Consumes: `EntryTranslationController.translate(): Promise<boolean>` and `lookup(text): Promise<boolean>`.
- Produces: translation and lookup actions that use `onClick={() => void ...}` while loading state remains controller-owned.

- [ ] Write a Vitest route-harness test with a never-resolving `translate()` promise; click “Translate article”, then click a router link and assert the destination renders without resolving translation.
- [ ] Run `npm run test:ci -- src/features/translation/reader/TranslationReaderControls.test.tsx` from `web/` and confirm the unfixed component fails the navigation assertion.
- [ ] Replace both long-running `clickAction` handlers with ordinary `onClick` handlers and retain `isLoading`, `isDisabled`, and controller state.
- [ ] Re-run the targeted test and confirm it passes.
- [ ] Search `web/src` for other `clickAction` handlers that await network requests; classify each match as fixed or safe.

### Task 2: Bounded selection translation API

**Files:**
- Modify: `src/translation/model.rs`
- Modify: `src/translation/service.rs`
- Modify: `src/translation/mod.rs`
- Modify: `src/api/translation.rs`
- Modify: `docs/openapi/translation-v2.json`
- Modify: `tests/translation_api.rs`

**Interfaces:**
- Produces: `TranslationService::translate_text(user_id: &str, text: &str) -> Result<TranslationTextResult, TranslationError>`.
- Produces: `POST /api/v2/plugins/translation/translate` with `{ "text": string }` and `{ translatedText, providerLabel, detectedSourceLocale, targetLocale }`.

- [ ] Add API tests proving authentication, CSRF, strict JSON, empty/control/8,001-character rejection, saved-provider use, and bounded success.
- [ ] Run `cargo test --test translation_api selection -- --nocapture` and confirm the new endpoint tests fail before implementation.
- [ ] Add `normalize_translation_text` with an 8,000-character maximum and the same control-character policy as lookup.
- [ ] For OpenAI call `translate` with the saved profile/prompts; for DeepLX reuse saved key/base URL and `translate_deeplx_text` chunking.
- [ ] Wrap the operation in the existing translation timeout, request rate limit, and per-user/global concurrency permits.
- [ ] Add the OpenAPI request/result schemas and endpoint responses.
- [ ] Re-run the targeted Rust API and service tests.

### Task 3: Reader selection floating popover

**Files:**
- Create: `web/src/features/translation/reader/ArticleSelectionPopover.tsx`
- Create: `web/src/features/translation/reader/ArticleSelectionPopover.test.tsx`
- Create: `web/src/features/translation/reader/articleSelection.ts`
- Create: `web/src/features/translation/reader/TranslationResultViews.tsx`
- Create: `web/src/features/translation/api/translation.test.ts`
- Create: `web/src/features/translation/model/useEntryTranslationController.test.tsx`
- Modify: `web/src/features/reader/components/ArticleReader.tsx`
- Modify: `web/src/features/translation/model/useEntryTranslationController.ts`
- Modify: `web/src/features/translation/api/translation.ts`
- Modify generated: `web/src/features/translation/api/translation.generated.ts`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/src/shared/i18n/i18n.ts`

**Interfaces:**
- Produces: `controller.translateSelection(text): Promise<boolean>`, `selectionResult`, `isTranslatingSelection`, and `clearSelectionTranslation()`.
- Consumes: an ASTRYX `Popover` anchored to a hidden one-pixel trigger positioned at the pointer coordinates.

- [ ] Generate the TypeScript contract with `npm run generate:reader-types` after updating OpenAPI.
- [ ] Add API-client validation and controller cancellation/reset behavior for selection translation.
- [ ] Add tests for a selection wholly inside the article, selection outside/crossing the article, the 200-character lookup threshold, the 8,000-character translation cap, Escape dismissal, and rendered result text.
- [ ] Wrap only the article reading surface with the selection popover trigger; preserve the native browser menu when there is no eligible selection or the selection exceeds 8,000 characters.
- [ ] Snapshot the trimmed selection on `contextmenu`; immediately look up selections of 200 characters or fewer and immediately translate selections of 201–8,000 characters.
- [ ] Let short selections switch between “Look up” and “Translate selection” inside the same floating result surface; never show a manual text input.
- [ ] Display pending/result/error state in a compact pointer-anchored panel using React text nodes; omit animation for this high-frequency action.
- [ ] Abort stale lookup/translation requests when the popover closes, the action changes, or the reader navigates to another entry.
- [ ] Run the targeted reader/popover/controller tests, typecheck, and production build.

### Task 4: Release and verification

**Files:**
- Modify: `Cargo.toml`
- Modify generated: `Cargo.lock`
- Modify: `web/package.json`
- Modify: `README.md`
- Modify: `web/e2e/admin-only-setup.spec.ts`
- Modify: `web/e2e/support/readerCategoryScenarios.ts`

**Interfaces:**
- Produces: release `v0.3.6` from `main`.

- [ ] Update Rust and web versions to `0.3.6`, refresh the lockfile, and add release notes covering navigation responsiveness and selection translation.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, web typecheck/tests/build, generated-contract checks, release-contract checks, and dependency/security audits.
- [ ] Verify the regression test red-green cycle by temporarily restoring `clickAction`, observing failure, restoring the fix, and observing pass.
- [ ] Review `git diff`, confirm no cache files or secrets are staged, then commit with a message explaining that an async React transition caused the regression and ordinary event handlers prevent recurrence.
- [ ] Push `main`, create and push annotated tag `v0.3.6`, then use `gh` to verify CI, Docker multi-arch manifest, Release assets, checksums, SBOM, and provenance.
