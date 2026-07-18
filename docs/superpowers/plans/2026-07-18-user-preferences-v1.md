# Raindrop User Preferences v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This repository is explicitly configured for inline main-Agent execution; do not dispatch subagents.

**Goal:** Persist and apply user-scoped theme, locale, Reader density, and reading font scale through a secure API and responsive ASTRYX settings flow.

**Architecture:** A normalized `user_preferences` row is keyed by `user_id`. A short user-locked transaction owns partial updates, `/api/v1/preferences` exposes the complete effective object, and the frontend uses generated wire types plus a preference runtime above the ASTRYX root Theme. A strictly validated local hint prevents the initial theme flash but never becomes an authentication or data authority.

**Tech Stack:** Rust 2024, Axum, SeaORM 1.1.19, SQLite/PostgreSQL/MySQL, React 19, TypeScript 7, Lingui 6.5, ASTRYX 0.1.6, Vitest 4, Playwright 1.61.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-user-preferences-v1-design.md` exactly.
- No new Rust or npm runtime dependency.
- Every repository operation receives an explicit authenticated user ID; the HTTP API accepts no owner ID.
- External input is strict and bounded; storage corruption is redacted.
- TypeScript wire DTOs come only from committed OpenAPI artifacts.
- Use ASTRYX `MoreMenu`, `Dialog`, `SegmentedControl`, `RadioList`, `Button`, `Stack`, and `Banner`; do not create generic replacements.
- Keep Reader usable while preferences load or fail.
- Keep new non-generated TypeScript/TSX and Rust modules focused and near 250 lines or less where the architecture permits.
- Do not touch `.superpowers/research/` or root `node_modules/`.
- Each completed task is committed and pushed to `feature/foundation-bootstrap` before the next task.

---

### Task 1: Portable preference schema

**Files:**
- Create: `src/db/migration/preferences.rs`
- Create: `src/db/entities/user_preference.rs`
- Create: `tests/preference_migrations.rs`
- Modify: `src/db/migration.rs`
- Modify: `src/db/entities.rs`

**Interfaces:**
- Produces `user_preferences(user_id, locale, theme_mode, layout_density, reading_font_scale, created_at, updated_at)`.
- Produces `user_preference::Model` consumed by Task 2.

- [x] **Step 1: Write the migration RED contract**

Create one shared contract with mandatory SQLite and opt-in PostgreSQL/MySQL entry points. The contract migrates twice, inserts a user and a valid preference row, rejects every invalid enum and scales `84`/`131`, rejects a second row for the same user, deletes the user, and proves cascade removal.

```rust
#[tokio::test]
async fn sqlite_preference_schema_contract() {
    let url = sqlite_contract_url("preference-contract.db");
    preference_schema_contract(url).await;
}
```

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test preference_migrations`

Expected: compile failure because `entities::user_preference` and `CreateUserPreferences` do not exist.

- [x] **Step 3: Implement the migration and entity**

Append `CreateUserPreferences` after `CreateOrganizationTables`. Use the user ID as the table primary key and foreign key with `ON DELETE CASCADE`. Add column checks with SeaQuery expressions:

```rust
.check(Expr::col(UserPreferences::Locale).is_in(["zh-CN", "en"]))
.check(Expr::col(UserPreferences::ThemeMode).is_in(["SYSTEM", "LIGHT", "DARK"]))
.check(Expr::col(UserPreferences::LayoutDensity).is_in([
    "COMPACT", "BALANCED", "SPACIOUS",
]))
.check(Expr::col(UserPreferences::ReadingFontScale).between(85, 130))
```

The entity model is:

```rust
pub struct Model {
    pub user_id: String,
    pub locale: String,
    pub theme_mode: String,
    pub layout_density: String,
    pub reading_font_scale: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
```

- [x] **Step 4: Verify schema contracts**

Run: `cargo test --locked --all-features --test preference_migrations --test organization_migrations --test database_migrations`

Expected: SQLite passes; PostgreSQL/MySQL skip only when their existing test URLs are absent.

- [x] **Step 5: Commit and push**

```bash
git add src/db/migration.rs src/db/migration/preferences.rs src/db/entities.rs src/db/entities/user_preference.rs tests/preference_migrations.rs
git commit -m "feat: add user preference storage"
git push origin feature/foundation-bootstrap
```

### Task 2: User-scoped preference repository

**Files:**
- Create: `src/preferences/mod.rs`
- Create: `src/preferences/types.rs`
- Create: `src/preferences/repository.rs`
- Create: `tests/preference_repository.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces `PreferenceRepository::get(user_id, default_locale)` and `PreferenceRepository::update(user_id, default_locale, patch)`.
- Produces `Locale`, `ThemeMode`, `LayoutDensity`, `UserPreferences`, `UpdateUserPreferences`, and `PreferenceError`.

- [x] **Step 1: Write repository RED tests**

Cover missing-row defaults, all single-field patches, complete patch, inclusive scale bounds, empty patch, two-user isolation, cascade behavior, invalid stored enum/scale as redacted corruption, and concurrent disjoint patches preserving both changes.

```rust
let updated = repository
    .update(
        USER_A_ID,
        Locale::En,
        UpdateUserPreferences {
            theme_mode: Some(ThemeMode::Dark),
            ..Default::default()
        },
    )
    .await?;
assert_eq!(updated.theme_mode, ThemeMode::Dark);
```

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test preference_repository`

Expected: compile failure because `raindrop::preferences` does not exist.

- [x] **Step 3: Implement typed validation and decoding**

Each enum owns exact storage/wire names. `UserPreferences::defaults(locale)` returns `SYSTEM`, `BALANCED`, and `100`. `UpdateUserPreferences::validate()` rejects an empty patch and values outside `85..=130`. `PreferenceError::CorruptStorage` contains no stored value.

- [x] **Step 4: Implement short user-locked transactions**

Lock the owning `users` row using the same backend-aware pattern as category creation. Select the preference row, apply the patch to stored values or defaults, then insert or update with database timestamps. Missing users return `NotFound`; public HTTP callers will only supply `CurrentUser.id`.

- [x] **Step 5: Verify repository and schema**

Run: `cargo test --locked --all-features --test preference_repository --test preference_migrations`

- [x] **Step 6: Commit and push**

```bash
git add src/lib.rs src/preferences/mod.rs src/preferences/types.rs src/preferences/repository.rs tests/preference_repository.rs
git commit -m "feat: add user preference repository"
git push origin feature/foundation-bootstrap
```

### Task 3: Preferences HTTP and OpenAPI contract

**Files:**
- Create: `src/api/preferences.rs`
- Create: `docs/openapi/preferences-v1.json`
- Create: `tests/preference_api.rs`
- Create: `tests/preference_openapi_contract.rs`
- Modify: `src/api/mod.rs`
- Modify: `src/api/routes.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Produces `GET/PATCH /api/v1/preferences`.
- Produces the committed OpenAPI artifact consumed by Task 4.

- [x] **Step 1: Write router RED tests**

Test authentication, CSRF precedence, `Accept-Language` defaults, complete GET shape, every valid patch field, empty and unknown objects, invalid enum/scale, per-user persistence, no-store headers, exact method fallback, trailing/unknown JSON fallback, and a separate per-user mutation budget.

- [x] **Step 2: Run and confirm RED**

Run: `cargo test --locked --all-features --test preference_api`

Expected: preference paths return SPA/404 because no router exists.

- [x] **Step 3: Implement the focused router**

Use strict request enums and map them to domain enums. Parse only the leading `Accept-Language` language range: strings starting with `zh` produce `Locale::ZhCn`; all other or malformed headers produce `Locale::En`. The PATCH handler extracts `CurrentUser`, then `CsrfGuard`, then `ApiJson`, checks `preferences_mutation_limiter`, and returns the full response.

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchPreferencesRequest {
    locale: Option<LocaleRequest>,
    theme_mode: Option<ThemeModeRequest>,
    layout_density: Option<LayoutDensityRequest>,
    reading_font_scale: Option<i32>,
}
```

- [x] **Step 4: Add OpenAPI and real-router drift tests**

The artifact contains exactly GET and PATCH operations, session/CSRF security, strict camelCase schemas, scale bounds, the shared stable error envelope, and no `userId`, timestamps, or storage strings beyond public enum values.

- [x] **Step 5: Verify API contracts**

Run: `cargo test --locked --all-features --test preference_api --test preference_openapi_contract --test session_security`

- [x] **Step 6: Commit and push**

```bash
git add src/api/preferences.rs src/api/mod.rs src/api/routes.rs src/app.rs docs/openapi/preferences-v1.json tests/preference_api.rs tests/preference_openapi_contract.rs
git commit -m "feat: expose user preferences api"
git push origin feature/foundation-bootstrap
```

### Task 4: Generated client and first-paint runtime

**Files:**
- Modify: `web/scripts/generate-reader-types.mjs`
- Create: `web/src/features/preferences/api/preferences.generated.ts`
- Create: `web/src/features/preferences/api/preferences.ts`
- Create: `web/src/features/preferences/api/preferences.test.ts`
- Create: `web/src/features/preferences/model/preferenceTypes.ts`
- Create: `web/src/features/preferences/model/preferenceCache.ts`
- Create: `web/src/features/preferences/model/preferenceCache.test.ts`
- Create: `web/src/features/preferences/model/PreferenceRuntime.tsx`
- Create: `web/src/features/preferences/model/PreferenceRuntime.test.tsx`
- Create: `web/public/theme-bootstrap.js`
- Modify: `web/index.html`
- Modify: `web/src/app/Providers.tsx`
- Modify: `web/src/shared/i18n/i18n.ts`

**Interfaces:**
- Produces `getPreferences()` and `patchPreferences(csrfToken, patch)`.
- Produces `usePreferenceRuntime()` with `{ preferences, apply, clearHint }`.

- [ ] **Step 1: Add generator, API, cache, and runtime RED tests**

Verify the new artifact participates in drift checking, malformed responses fail closed, PATCH sends CSRF and only changed fields, cache parsing rejects unknown/missing/out-of-range values, apply updates ASTRYX theme mode/locale/density/scale, and clear removes the hint.

- [ ] **Step 2: Run and confirm RED**

Run: `cd web && npm run test:ci -- src/features/preferences`

- [ ] **Step 3: Generate the strict wire client**

Add `preferences-v1.json` to the artifact list and generate `UserPreferences`, `PatchUserPreferencesRequest`, enum aliases, and runtime validators. Handwritten API functions reuse `apiRequest` and `invalidResponseError`.

- [ ] **Step 4: Implement the non-sensitive cache and runtime**

Cache shape:

```ts
interface PreferenceHintV1 {
  schemaVersion: 1
  preferences: UserPreferences
}
```

Only this exact object is stored under `raindrop.preferences.v1`. `theme-bootstrap.js` independently validates exact keys and values before setting presentation attributes. `PreferenceRuntime` maps uppercase wire modes to the ASTRYX `Theme` prop and applies `activateLocale` in one state transition.

- [ ] **Step 5: Verify generated and runtime contracts**

Run: `cd web && npm run generate:reader-types && npm run check:reader-types && npm run typecheck && npm run test:ci -- src/features/preferences src/shared/i18n/i18n.test.ts`

- [ ] **Step 6: Commit and push**

```bash
git add web/scripts/generate-reader-types.mjs web/src/features/preferences web/public/theme-bootstrap.js web/index.html web/src/app/Providers.tsx web/src/shared/i18n/i18n.ts
git commit -m "feat: add preference runtime"
git push origin feature/foundation-bootstrap
```

### Task 5: ASTRYX settings workflow and Reader application

**Files:**
- Create: `web/src/features/preferences/model/usePreferencesController.ts`
- Create: `web/src/features/preferences/model/usePreferencesController.test.tsx`
- Create: `web/src/features/preferences/components/PreferencesDialog.tsx`
- Create: `web/src/features/preferences/components/PreferencesDialog.test.tsx`
- Modify: `web/src/features/reader/ReadyPage.tsx`
- Modify: `web/src/features/reader/routes/ReaderRoutes.tsx`
- Modify: `web/src/features/reader/layout/ReaderShell.tsx`
- Modify: `web/src/features/reader/components/SourceTree.tsx`
- Modify: `web/src/features/reader/components/ReaderToolbar.tsx`
- Modify: `web/src/features/reader/components/EntryQueue.tsx`
- Modify: `web/src/features/reader/components/ArticleReader.tsx`
- Modify: `web/src/features/reader/reader.css`
- Modify: `web/src/shared/i18n/i18n.ts`
- Modify focused Reader tests beside those modules.

**Interfaces:**
- Produces a Settings and Sign out ASTRYX `MoreMenu` in the source toolbar.
- Applies preference density to source/entry lists and reading scale to article typography.

- [ ] **Step 1: Write controller/Dialog/Reader RED tests**

Cover parallel preference load, non-blocking load failure, successful save, CSRF, save failure rollback, unauthenticated callback, draft preservation, cancel, focus restoration, MoreMenu actions, density mapping, article scale, locale copy, and compact Dialog containment contract.

- [ ] **Step 2: Run and confirm RED**

Run: `cd web && npm run test:ci -- src/features/preferences web/src/features/reader`

- [ ] **Step 3: Implement the preference controller**

Load once when `ReadyPage` mounts. Controller state is always initialized from the runtime hint/defaults so Reader rendering never waits. `save(draft)` computes a field-level patch, returns early when unchanged, calls PATCH once, applies the response, and keeps a stable inline error on failure.

- [ ] **Step 4: Implement the ASTRYX Dialog and MoreMenu**

Use one `Dialog purpose="form"` with the four controls from the spec and Cancel/Save footer. Replace the direct logout icon with:

```tsx
<MoreMenu
  label={i18n._("common.menu")}
  size="lg"
  items={[
    { label: i18n._("preferences.open"), onClick: onOpenPreferences },
    { type: "divider" },
    { label: i18n._("common.logout"), onClick: onLogout },
  ]}
/>
```

Settings and Sign out remain secondary actions; Manage categories and Add subscription stay directly visible.

- [ ] **Step 5: Apply density and typography without custom controls**

Map density to existing ASTRYX list props. Set Reader article typography through `--raindrop-reading-scale` and `calc()` while retaining the 72-character measure and CJK line height. Do not change toolbar/control font size or touch target size.

- [ ] **Step 6: Verify frontend behavior**

Run: `cd web && npm run check:reader-types && npm run typecheck && npm run test:ci && npm run build`

- [ ] **Step 7: Commit and push**

```bash
git add web/src/features/preferences web/src/features/reader web/src/shared/i18n/i18n.ts
git commit -m "feat: add reader preference settings"
git push origin feature/foundation-bootstrap
```

### Task 6: Production browser verification and task state

**Files:**
- Modify: `web/e2e/support/readerApiFixture.ts`
- Modify: `web/e2e/reader-workspace.spec.ts`
- Create: `web/e2e/support/readerPreferenceScenarios.ts`
- Modify: `tasks/todo.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/user-preferences-v1-report.md`

**Interfaces:**
- Produces deterministic browser evidence and authoritative progress state.

- [ ] **Step 1: Extend the Reader fixture**

Support preference GET/PATCH with mutable per-user state, strict CSRF capture, and a controllable PATCH failure. Keep bootstrap, session, embedded production assets, and application routing real.

- [ ] **Step 2: Add viewport scenarios**

At 1280×800 change all four values, save, reload, and prove persistence. At 900×800 verify MoreMenu and focus restoration. At 390×844 and 360×800 verify Dialog containment, locale copy, no horizontal overflow, and theme attributes. Verify a PATCH failure preserves the draft and effective previous preferences.

- [ ] **Step 3: Run local agent-browser on the production binary**

Use a temporary SQLite instance, create/login a user, switch dark/light/system and zh-CN/en, reload, and record theme attribute, locale, density, reading scale, console/errors, and viewport containment. Do not retain cookies or database files.

- [ ] **Step 4: Run the bounded final gates once**

```bash
cd web
npm run check:reader-types
npm run typecheck
npm run test:ci
npm run build
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --project reader-1280x800 --project reader-900x800 --project reader-390x844 --project reader-360x800
cd ..
cargo test --locked --all-features
git diff --check
```

- [ ] **Step 5: Update task and report state**

Record exact counts and existing advisories. Do not mark the combined sorting/reading-cursor/user-settings line complete unless sorting and cursor are also implemented; add a completed nested user preference line while leaving the parent unchecked. Leave OIDC, AI/plugin/MCP, OPML, and release items unchecked.

- [ ] **Step 6: Commit and push**

```bash
git add web/e2e/support/readerApiFixture.ts web/e2e/reader-workspace.spec.ts web/e2e/support/readerPreferenceScenarios.ts tasks/todo.md tasks/plan.md docs/superpowers/plans/2026-07-18-user-preferences-v1.md docs/superpowers/specs/2026-07-18-user-preferences-v1-design.md
git add -f .superpowers/sdd/user-preferences-v1-report.md
git commit -m "test: verify user preferences"
git push origin feature/foundation-bootstrap
```

## Plan self-review

- Spec coverage: schema, transactions, API, generated DTOs, first-paint hint, ASTRYX UI, density/typography application, mobile containment, failure rollback, cross-user persistence, and production verification each map to a task.
- DDIA consistency: the user row is the serialization boundary; preference storage remains normalized and authoritative; localStorage is a presentation cache only.
- Security consistency: no custom code fields, no owner parameter, strict input/storage decoding, CSRF, limiter, no-store, no auth material in cache.
- Type consistency: `locale`, `themeMode`, `layoutDensity`, and `readingFontScale` are identical across domain, OpenAPI, generated TypeScript, runtime, and UI.
- Placeholder scan: no `TBD`, deferred implementation placeholder, undefined later interface, or generic error-handling instruction remains.
- Scope exclusions: sorting, read cursor, accent presets, settings import/export, OIDC, AI/plugin/MCP, and release remain separate additive plans.
