# Raindrop User Preferences v1 Design

## Objective

Deliver authenticated, user-scoped appearance preferences that persist across browsers and database backends without blocking RSS reading. Version 1 covers `LIGHT | DARK | SYSTEM`, `zh-CN | en`, Reader density, and reading font scale. The settings UI uses ASTRYX components and fits the existing desktop, medium, and compact Reader shells.

Success means a user can open Settings, change the four preferences, save once, reload the production binary, and see the same effective theme, locale, density, and article typography. Another user on the same instance must keep independent values.

## Reference absorption and improvements

CommaFeed stores a one-to-one `USERSETTINGS` record and exposes display preferences through a dedicated settings page. Its client applies optimistic changes immediately, but every control sends a full settings object. Raindrop keeps the useful one-row-per-user model and visible display controls while changing the unsafe or high-write parts:

- PATCH carries only changed fields instead of resending the full object.
- A single Save action commits the draft, avoiding one write per click.
- Save failure keeps the dialog open and restores the previous effective runtime preferences.
- Custom CSS and custom JavaScript are excluded; preferences cannot become an XSS or arbitrary-code surface.
- Server storage is authoritative. A strictly validated local hint exists only to prevent an initial theme flash.
- The original article remains readable while preferences load or fail.

## Assumptions and scope

- Existing session cookies, `CurrentUser`, `CsrfGuard`, `ApiJson`, and no-store response helpers remain the security boundary.
- Missing persisted preferences resolve to defaults without inserting a row: request/browser locale, `SYSTEM`, `BALANCED`, and `100` percent reading scale.
- The authenticated UI is the first persistence surface. Setup and login language switches remain browser-local until a user signs in.
- Accent presets, reading order, mark-all-read behavior, profile editing, settings JSON import/export, OIDC, and administrator policy are separate additive slices.
- No new Rust or npm runtime dependency is introduced.

## Tech stack

- Rust 2024, Axum, SeaORM 1.1.19, SeaORM Migration, SQLite/PostgreSQL/MySQL.
- React 19, TypeScript 7, Lingui 6.5, ASTRYX 0.1.6, Vite 8, Vitest 4, Playwright 1.61.
- Existing warm-paper Raindrop token layer and Charter/CJK serif Reader typography.

## Data model

Create one normalized record per user:

```text
user_preferences
  user_id             VARCHAR(36) PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE
  locale              VARCHAR(8)  NOT NULL CHECK IN ('zh-CN', 'en')
  theme_mode          VARCHAR(16) NOT NULL CHECK IN ('SYSTEM', 'LIGHT', 'DARK')
  layout_density      VARCHAR(16) NOT NULL CHECK IN ('COMPACT', 'BALANCED', 'SPACIOUS')
  reading_font_scale  INTEGER     NOT NULL CHECK BETWEEN 85 AND 130
  created_at          TIMESTAMP WITH TIME ZONE NOT NULL
  updated_at          TIMESTAMP WITH TIME ZONE NOT NULL
```

The user primary key is also the ownership boundary; there is no public preference ID to guess. The record is normalized because every field is independently patched and queried together. A JSON blob would weaken schema-on-write validation and make three-database evolution harder.

Reads are point lookups by primary key. Writes run in a short transaction, lock the owning user row, read the current record or supplied defaults, apply only present fields, then insert or update the full validated record. Concurrent patches to different fields serialize under the user lock rather than losing a field through read-modify-write races.

No preference write participates in a Feed transaction, lifecycle outbox, or Reader entry-state transaction.

## Domain contract

```rust
pub enum Locale { ZhCn, En }
pub enum ThemeMode { System, Light, Dark }
pub enum LayoutDensity { Compact, Balanced, Spacious }

pub struct UserPreferences {
    pub locale: Locale,
    pub theme_mode: ThemeMode,
    pub layout_density: LayoutDensity,
    pub reading_font_scale: i32,
}

pub struct UpdateUserPreferences {
    pub locale: Option<Locale>,
    pub theme_mode: Option<ThemeMode>,
    pub layout_density: Option<LayoutDensity>,
    pub reading_font_scale: Option<i32>,
}
```

All enums use exact stable wire strings. `reading_font_scale` accepts inclusive `85..=130`; the UI exposes `90`, `100`, `110`, and `120` presets. Empty patches are invalid. Domain errors never include raw request bodies or stored corrupt values.

## HTTP and OpenAPI contract

```text
GET   /api/v1/preferences
PATCH /api/v1/preferences
```

`GET` requires an active session and returns effective preferences. When no row exists, the handler supplies `zh-CN` if `Accept-Language` begins with `zh`, otherwise `en`; all other defaults are stable.

`PATCH` requires an active session, same-origin CSRF, a per-user preferences mutation limiter, `Content-Type: application/json`, and a strict object with at least one known field. It returns the complete persisted preference object.

Response shape:

```json
{
  "locale": "zh-CN",
  "themeMode": "SYSTEM",
  "layoutDensity": "BALANCED",
  "readingFontScale": 100
}
```

Both operations return `Cache-Control: no-store` and `Pragma: no-cache`. Stable status coverage is `200`, `401`, `403`, `405`, `422`, `429`, and `500`. Unknown preference paths return JSON 404 rather than the SPA shell. The committed artifact is `docs/openapi/preferences-v1.json`, and generated TypeScript is the only frontend wire DTO source.

## Frontend runtime contract

`PreferenceRuntimeProvider` owns the effective preference state above the root ASTRYX `Theme`. It maps wire `SYSTEM | LIGHT | DARK` to ASTRYX `system | light | dark`, activates Lingui locale, writes `data-raindrop-density`, and sets `--raindrop-reading-scale` on `<html>`.

A static same-origin `/theme-bootstrap.js` runs from `<head>` before the React entry. It reads `raindrop.preferences.v1` from localStorage, validates `schemaVersion`, and sets only `data-theme`, `lang`, `data-raindrop-density`, and `--raindrop-reading-scale`. It contains no dynamic code execution and no authentication data. React repeats strict validation, removes malformed hints, and replaces the hint after an authenticated GET/PATCH. Logout removes the hint.

The preference request runs alongside Reader loading. A failure does not block sources, queue, article content, keyboard navigation, read state, or Feed refresh.

## Settings UI and UX

The 240px source toolbar keeps Manage categories and Add subscription visible. ASTRYX `MoreMenu` replaces the direct Sign out icon and contains two secondary actions: Settings and Sign out. This preserves three 44px toolbar hit targets without crossing the source divider.

Settings uses one ASTRYX `Dialog purpose="form"`:

- Appearance: `SegmentedControl` for System, Light, Dark.
- Language: `SegmentedControl` for 中文 and English.
- Density: `SegmentedControl` for Compact, Balanced, Spacious.
- Reading size: `RadioList` for 90%, 100%, 110%, and 120%, with 100% preselected by default.
- Footer: Cancel and Save changes.

The Dialog has an accessible name, a bounded scroll area, no nested dialog, no custom animation, and restores focus to the MoreMenu trigger path. At 390×844 and 360×800 it stays inside the viewport without document horizontal overflow. Saving disables repeated submission. Validation or network failure stays inline and preserves the draft.

Density maps directly to ASTRYX `TreeList` and `List` density values. Reading scale affects only article title/body/metadata rhythm, not control text or 44px targets. The article measure remains about 72 Latin characters and CJK line height remains at least 1.7.

## Threat model

| Boundary | Abuse case | Control |
| --- | --- | --- |
| Preferences PATCH | Cross-site mutation | `CsrfGuard`, same-origin contract, session cookie |
| User ownership | One user changes another user | User ID comes only from `CurrentUser`; no public owner parameter |
| JSON input | Unknown fields, invalid enums, huge scale | `deny_unknown_fields`, typed enums, inclusive numeric bound, global body limit |
| Mutation volume | Repeated preference writes | Dedicated per-user limiter and one Save action |
| Database corruption | Invalid stored enum or scale reaches UI | Strict storage decoder returns redacted internal error |
| Local hint | Tampered localStorage changes application behavior | Strict allowlist validator; values affect presentation only; server replaces after login |
| XSS | Preference becomes executable CSS/JS | No custom CSS, custom JS, HTML, URL, or free-form style fields |
| Information disclosure | Preferences leak through caches/errors | no-store responses; no raw values in internal errors |

## Commands

```bash
cargo test --locked --all-features --test preference_migrations --test preference_repository --test preference_api --test preference_openapi_contract
npm --prefix web run generate:reader-types
npm --prefix web run check:reader-types
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cd web && PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --project reader-1280x800 --project reader-390x844 --project reader-360x800
cargo test --locked --all-features
git diff --check
```

## Project structure

```text
src/preferences/                         domain repository and validation
src/api/preferences.rs                  authenticated GET/PATCH routes
src/db/migration/preferences.rs         portable schema
src/db/entities/user_preference.rs      SeaORM entity
docs/openapi/preferences-v1.json        committed public contract
web/src/features/preferences/api/       generated and handwritten API boundary
web/src/features/preferences/model/     runtime state, cache, controller
web/src/features/preferences/components settings dialog and focused controls
web/public/theme-bootstrap.js            first-paint non-sensitive hint
```

## Testing strategy

- Migration contracts: SQLite mandatory, PostgreSQL/MySQL opt-in through existing test URLs; enforce PK/FK/check constraints and cascade delete.
- Repository contracts: defaults, partial updates, concurrent disjoint patches, bounds, user isolation, corrupt storage, delete cascade.
- API contracts: auth/CSRF precedence, empty/unknown/invalid patch, no-store, method/path fallback, rate limit, per-user persistence.
- OpenAPI drift: exact route surface, schemas, security, stable responses, real-router response validation.
- Frontend: strict generated response validation, local hint rejection, runtime application, save rollback, logout cleanup, density/scale rendering.
- Browser: theme reload persistence, locale switch, mobile Dialog containment, focus restoration, no horizontal overflow, empty console/page errors.

## Boundaries

- Always: explicit user scope, parameterized queries, short transactions, no-store preference responses, generated wire DTOs, ASTRYX controls, localized copy, browser verification.
- Internally self-review before implementation because the user delegated review and confirmation to the main agent.
- Never: custom CSS/JS, auth material in localStorage, preference loading that blocks original RSS content, hand-written duplicate wire types, nested dialogs, unrelated refactors, new dependency without necessity.

## Success criteria

- Two users can persist different values for all four preferences on SQLite, PostgreSQL, and MySQL contracts.
- Invalid enum, unknown field, empty patch, out-of-range font scale, missing auth, and missing CSRF fail with stable public responses.
- Production web reload starts in the cached theme without a visible opposite-theme flash, then reconciles with the authenticated server value.
- Theme, locale, density, and reading scale update the real Reader and survive reload.
- Settings remains usable and contained at 1280×800, 900×800, 390×844, and 360×800.
- Existing Reader, category, RSS, setup, login, and embedded-web tests remain green.

## Internal self-review

- DDIA: a normalized one-row record matches the point-read/partial-write access pattern; the owning user row serializes concurrent patches; constraints and strict decoding protect schema-on-write; no second source of truth is introduced.
- API: additive `/api/v1/preferences`, stable complete response, partial PATCH, explicit security/error surface, committed OpenAPI artifact.
- Security: presentation-only local hint, no executable customization, current-user ownership, CSRF, rate limit, redacted corruption path.
- UI: one focused ASTRYX Dialog, no toolbar overflow, no custom generic controls, compact/mobile containment, original content never waits on settings.
- Scope: accent presets, reading order, settings import/export, OIDC, admin, AI/plugin/MCP, and release remain explicit later slices rather than placeholders here.
- Open questions: none; decisions are complete for v1.
