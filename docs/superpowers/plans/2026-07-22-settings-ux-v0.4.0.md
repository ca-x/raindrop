# Settings UX and v0.4.0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close completed subscription management, add an explicit mark-subscription-read action, make Settings feel structured and stable, add an About/version surface, and publish the verified result as `v0.4.0`.

**Architecture:** Keep subscription behavior inside the existing management dialog and preserve the optional organization step, but close the dialog after its successful final submit. Keep the Settings dialog shell and ASTRYX controls; move scrolling to the active content panel so the navigation stays fixed, add semantic navigation icons, render destructive font actions as compact icon buttons, and inject the Web version from `web/package.json` at build time.

**Tech Stack:** React 19, TypeScript 7, ASTRYX Design Core, Vite 8, Vitest, Playwright, Rust/Cargo release workflow.

## Global Constraints

- Apply `$emil-design-eng`: motion must be purposeful, under 300 ms, interruptible, and reduced-motion safe.
- Keep all pointer targets at least 44 by 44 CSS pixels on touch layouts.
- Core settings navigation keeps visible text; icons support recognition and never replace the label.
- A successful final subscription submit closes the management dialog; failures preserve the current form state.
- The selected subscription exposes a confirmed “mark all read” action backed by the existing snapshot-safe bulk-read API.
- Atom/RSS parsing is unchanged because the reported Atom source is valid and has since refreshed successfully.
- Release version is exactly `0.4.0`; the annotated Git tag is exactly `v0.4.0`.

---

### Task 1: Pin the subscription completion behavior

**Files:**
- Modify: `web/src/features/reader/components/SubscriptionManagementDialog.test.tsx`
- Modify: `web/src/features/reader/components/SubscriptionManagementDialog.tsx`

**Interfaces:**
- Consumes: `onOpenChange(isOpen: boolean)` and the existing two-step `onAdd`/`onUpdate` workflow.
- Produces: successful `finishAdd` calls `onOpenChange(false)`; failed updates keep the dialog open.

- [ ] **Step 1: Write the failing test**

Add a harness that creates a subscription, clicks `Continue`, clicks `Finish adding`, and asserts `onOpenChange(false)` instead of a reset to the Feed URL form. Add a failure case whose `onUpdate` returns `false` and assert the organization form remains visible.

- [ ] **Step 2: Run the focused test to verify it fails**

Run: `npm test -- --run src/features/reader/components/SubscriptionManagementDialog.test.tsx` from `web/`.

Expected: the success test fails because the current code clears `pendingSubscription` and `url` without closing.

- [ ] **Step 3: Implement the minimal transition**

Pass the dialog's existing `close()` callback into `SubscriptionPanel` as `onComplete`. Replace the successful tail of `finishAdd`:

```ts
if (!saved) return
props.onComplete()
```

Do not close on validation, create, or update failure.

- [ ] **Step 4: Re-run the focused test**

Run: `npm test -- --run src/features/reader/components/SubscriptionManagementDialog.test.tsx` from `web/`.

Expected: all tests in the file pass.

### Task 2: Restructure Settings and add About

**Files:**
- Modify: `web/src/features/preferences/components/PreferencesDialog.tsx`
- Modify: `web/src/features/preferences/components/PreferencesDialog.test.tsx`
- Modify: `web/src/features/preferences/components/AppearancePreferencesForm.tsx`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: `web/vite.config.ts`
- Create: `web/src/version.d.ts`

**Interfaces:**
- Consumes: `__RAINDROP_VERSION__` defined by Vite from `web/package.json`.
- Produces: `PreferencesTab = "personal" | "reading" | "plugins" | "about"`, a fixed settings navigation rail, an independently scrolling content panel, compact font delete actions, and an About panel showing `v${__RAINDROP_VERSION__}`.

- [ ] **Step 1: Write the failing component tests**

Add tests that select About and assert `Raindrop` plus `v0.4.0`, assert settings navigation items contain decorative SVG icons while retaining their accessible text, and assert a font delete action is icon-only with the full font-specific accessible name.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `npm test -- --run src/features/preferences/components/PreferencesDialog.test.tsx` from `web/`.

Expected: About is absent and the font delete action still has long visible text.

- [ ] **Step 3: Inject the version and implement the About panel**

Read `web/package.json` in `web/vite.config.ts` and define:

```ts
define: {
  __RAINDROP_VERSION__: JSON.stringify(packageJson.version),
}
```

Declare `__RAINDROP_VERSION__` in `web/src/version.d.ts`. Render an About navigation item and a simple product/version panel with selectable version text.

- [ ] **Step 4: Add icons without hiding labels**

Extend `SettingsNavButton` with an `icon: ReactNode` prop. Render a small icon container beside the label/description and use stable inline SVGs for Personal and Reading plus ASTRYX `wrench` and `info` icons for Plugins and About.

- [ ] **Step 5: Make the navigation fixed and content independently scrollable**

Set `LayoutContent isScrollable={false}`. Give `.reader-settings-layout` a definite block size and `min-block-size: 0`; keep `.reader-settings-nav` in its grid column and apply `overflow-y: auto`; apply `overflow-y: auto`, `overscroll-behavior: contain`, and the existing scrollbar styling to `.reader-preferences-panel`. On mobile, keep the navigation as the fixed horizontal row and scroll only the panel.

- [ ] **Step 6: Replace verbose font deletion with a compact icon action**

Use ASTRYX `IconButton` with a local trash SVG. Keep `preferences.deleteCustomFont` as the full accessible label and add a shorter `preferences.deleteFontAction` tooltip. Keep the row horizontal on mobile, truncate long font names, and preserve the global 44 px touch target.

- [ ] **Step 7: Re-run focused unit tests and type checking**

Run from `web/`:

```bash
npm test -- --run src/features/preferences/components/PreferencesDialog.test.tsx src/features/reader/components/SubscriptionManagementDialog.test.tsx
npm run typecheck
```

Expected: all focused tests pass and TypeScript reports no errors.

### Task 3: Add a subscription-level mark-all-read action

**Files:**
- Modify: `web/src/features/reader/components/SubscriptionEditDialog.tsx`
- Modify: `web/src/features/reader/components/SubscriptionEditDialog.test.tsx`
- Modify: `web/src/features/reader/layout/ReaderShell.tsx`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify: `web/src/features/reader/reader.css`

**Interfaces:**
- Consumes: `ReaderController.markCurrentSourceRead`, `ReaderController.isMarkingRead`, and the existing top-level `MarkReadDialog`.
- Produces: an explicit, icon-assisted action in the selected subscription dialog that opens the existing snapshot confirmation and marks only entries at or before the confirmed generation.

- [ ] **Step 1: Write the failing dialog and workspace tests**

Add `onRequestMarkRead` and `isMarkingRead` expectations to `SubscriptionEditDialog.test.tsx`: clicking “Mark all read” calls the request callback and the action disables while pending. Extend `ReaderWorkspace.test.tsx` so the selected-feed edit dialog opens the existing `MarkReadDialog`, confirms, and calls `controller.markCurrentSourceRead`.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run from `web/`:

```bash
npm test -- --run src/features/reader/components/SubscriptionEditDialog.test.tsx src/features/reader/ReaderWorkspace.test.tsx
```

Expected: the edit dialog has no mark-read action and does not open the confirmation.

- [ ] **Step 3: Implement the explicit action**

Add a non-destructive action section above the delete zone. Use the existing `checkDouble` icon, a visible “Mark all read” label, explanatory copy about the stable snapshot, and `onRequestMarkRead`. Wire it in `ReaderShell` to `setIsMarkReadOpen(true)` and pass `controller.isMarkingRead` for loading/disabled state. Reuse the existing `MarkReadDialog`; do not add a second confirmation implementation.

- [ ] **Step 4: Re-run the focused tests**

Run the command from Step 2.

Expected: all focused tests pass.

### Task 4: Verify the rendered interaction

**Files:**
- Modify: `web/e2e/reader-workspace.spec.ts`
- Modify when the accepted visuals change: `docs/assets/screenshots/reader-desktop.png`
- Modify when the accepted visuals change: `docs/assets/screenshots/reader-mobile.png`

**Interfaces:**
- Consumes: the Settings dialog and subscription fixture APIs.
- Produces: browser-level proof that Settings navigation stays in place while content scrolls, About exposes the version, and successful subscription completion closes the dialog.

- [ ] **Step 1: Extend Playwright coverage**

In the wide settings scenario, capture the navigation bounding box, scroll the settings content panel, assert the navigation top is unchanged, open About, and assert `v0.4.0`. In the subscription fixture, complete the add workflow and assert the management dialog is hidden.

- [ ] **Step 2: Run the targeted browser scenario**

Run from `web/`: `npx playwright test e2e/reader-workspace.spec.ts --project=chromium`.

Expected: the target scenario passes with no horizontal overflow and the navigation remains visible.

- [ ] **Step 3: Review desktop and mobile screenshots**

Open the generated screenshots and verify: fixed navigation, readable hierarchy, concise delete controls, 44 px touch targets, no clipped footer, no hover/selected color collision, and reduced-motion behavior unchanged.

### Task 5: Version, verify, commit, and publish `v0.4.0`

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `web/package.json`
- Modify: `web/package-lock.json`
- Modify: version assertions found by `rg -n '0\.3\.9|v0\.3\.9'` in release tests and docs
- Modify: `README.md`

**Interfaces:**
- Consumes: the green implementation from Tasks 1-3.
- Produces: commit on `main`, annotated tag `v0.4.0`, GitHub Release assets, and multi-architecture images.

- [ ] **Step 1: Synchronize version fields**

Set every shipped version to `0.4.0`, update release assertions, and add concise `v0.4.0` release notes covering Settings UX, About/version display, and subscription completion.

- [ ] **Step 2: Run complete verification**

Run the repository's Web unit tests, typecheck/build, Playwright suite, `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --all-targets --all-features`. Run the embedded release gates required by `web/package.json`.

- [ ] **Step 3: Re-read the release gate**

Confirm `git status --short --branch -uall`, `git rev-parse HEAD`, version alignment, generated Web assets, `raindrop --version`, and that only the known Vitest cache remains untracked.

- [ ] **Step 4: Commit and push**

Stage only intended files and commit with `release: v0.4.0`. Push `main`, create annotated tag `v0.4.0`, re-read the tag target, and push the tag.

- [ ] **Step 5: Verify public release surfaces**

Poll GitHub Actions as structured JSON. Confirm the GitHub Release contains five platform archives plus `SHA256SUMS`; verify checksums beyond file size; confirm GHCR and Docker Hub tags include `0.4.0`, `v0.4.0`, `0.4`, `latest`, and the commit SHA with amd64/arm64 manifests.

## Self-Review

- Spec coverage: subscription closure, subscription-level mark-all-read, fixed settings navigation, icon-assisted navigation, compact font deletion, About/version, and `v0.4.0` publishing are all assigned to tasks.
- Scope: Atom parsing and historical RSS backfill are explicitly excluded from this release because neither is a confirmed defect in the requested UI batch.
- Type consistency: the only new public frontend symbol is `__RAINDROP_VERSION__: string`; `PreferencesTab` includes `about` everywhere it is consumed.
- Placeholder scan: no deferred implementation placeholders remain.
