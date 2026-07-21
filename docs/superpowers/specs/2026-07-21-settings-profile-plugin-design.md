# Spec: Settings profile and plugin management

## Objective

Redesign Settings around user tasks instead of implementation details. Personal settings must let the signed-in user edit a display nickname and email while keeping the login username read-only. Plugin settings must start with an extensible plugin list whose first-class responsibilities are AI Provider management, AI-assisted reading, and translation.

## Tech Stack

- Rust 2024, Axum, SeaORM, SQLite/PostgreSQL/MySQL migrations.
- React 19, TypeScript, ASTRYX 0.1.6, Lingui, Vitest, Playwright.
- Session-cookie authentication, current CSRF guard, and the existing per-user settings mutation limiter.

## Commands

- Generate wire types: `cd web && npm run generate:reader-types`
- Typecheck: `cd web && npm run typecheck`
- Frontend tests: `cd web && npm run test:ci`
- Production web build: `cd web && npm run build`
- Rust format: `cargo fmt --check`
- Rust tests: `cargo test --locked`
- Release binary: `cargo build --release --locked`

## Project Structure

- `src/api/profile.rs`: authenticated v2 profile HTTP boundary.
- `src/auth/`: profile normalization, current-user load/update, and persistence errors.
- `src/db/migration/`: nullable display-name migration for existing installations.
- `docs/openapi/profile-v2.json`: committed profile wire contract.
- `web/src/features/profile/`: generated types, API client, and controller.
- `web/src/features/preferences/components/`: settings navigation, profile form, reading form, and plugin list/detail flow.
- `web/src/features/ai/settings/`: separate AI Provider and AI Assistant detail panels.
- `src/translation/` and `web/src/features/translation/`: user-scoped translation configuration, OpenAI/DeepLX engine selection, connection test, and article translation.

## Code Style

```rust
pub async fn update_profile(
    database: &DatabaseConnection,
    user_id: &str,
    patch: UpdateProfile,
) -> Result<UserProfile, ProfileError> {
    // Normalize once at the trust boundary, then persist only the current user.
}
```

Use strict request DTOs with `deny_unknown_fields`, additive API types, user-scoped repository updates, controlled component drafts, exact CSS transition properties, and no animation on settings navigation or list traversal.

## Testing Strategy

- API tests cover authentication, CSRF, strict JSON validation, nickname/email normalization, clearing fields, duplicate email conflicts, user isolation, cache headers, and rate limiting.
- Migration contracts cover the new nullable `users.display_name` column on all configured databases.
- Frontend controller tests cover load/save/error/authentication behavior and authoritative profile updates.
- Component tests cover editable nickname/email, read-only username, plugin list-first behavior, AI detail navigation, and separated save scopes.
- Browser checks cover wide, 900px, 390px, 375px, 360px, landscape, keyboard focus, reduced motion, safe-area padding, and horizontal overflow.

## Boundaries

- Always: validate profile input server-side, require authentication and CSRF for mutation, update only the current user, keep email unique, and preserve existing preference/AI save contracts.
- Ask first: changing login usernames, password flows, roles, or adding third-party plugin installation.
- Never: expose password hashes or credentials, make username editable in this slice, place Provider creation on the top-level plugin page, or use `transition: all`.

## Success Criteria

- Desktop Settings uses a substantially wider, viewport-bounded layout with stable navigation and content regions; mobile remains single-column without horizontal scrolling.
- Personal settings expose editable nickname and email fields plus an explicitly read-only login username.
- Saved nickname becomes the reader's visible account label; clearing it falls back to username.
- Email can be set or cleared, invalid input is rejected without leaking values, and duplicate email returns a stable conflict.
- Plugins opens to a list containing three built-in rows: AI Provider, AI Assistant, and Translation.
- AI Provider owns OpenAI-compatible endpoint/model/credential management and is reusable by the other plugins.
- AI Assistant owns article summary and synthesis. Translation controls are absent from this detail; keyword query and retrieval remain part of this plugin's future capability boundary and are not represented by non-functional controls.
- Translation owns full-article translation and word lookup, and selects exactly one engine: OpenAI or DeepLX.
- OpenAI translation selects an enabled OpenAI-compatible Provider. DeepLX exposes identity, connection, optional API key, Base URL, connection test, and target language inside the Translation detail rather than as a top-level plugin.
- OpenAI translation offers six useful prompt profiles: general, technical documentation, literary, academic, business/finance, and social/news. Advanced users may provide bounded custom system and translation prompt templates using `{{to}}` and `{{text}}`; DeepLX does not show irrelevant prompt controls.
- A DeepLX Base URL may contain one `{{apiKey}}` placeholder, for example `https://api.deeplx.org/{{apiKey}}/translate`. The saved key is URL-encoded into that placeholder and is not also sent as a Bearer header. Without the placeholder, a saved key is sent as a Bearer credential.
- Saved API keys are encrypted and never returned; connection tests use either a one-request draft key or the saved key, and translated article content is returned as escaped text/Markdown data.
- Word lookup accepts a short bounded text selection or manual input. DeepLX returns a direct translation; OpenAI may additionally return a concise definition and bilingual examples. The lookup response remains typed data and never renders upstream HTML.
- Full-article results are segment-addressed typed text. The original article DOM remains intact while translations are inserted as sibling nodes, supporting translation-only, bilingual, hover-reveal, and side-by-side reading modes.
- Touch targets remain at least 44px, focus is visible, hover-only styling is gated to fine pointers, and reduced motion removes transform feedback where appropriate.

## Open Questions

None. The requested scope explicitly authorizes breaking the unreleased mixed AI/translation settings contract and replacing it with the three-plugin model above.
