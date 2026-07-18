# Raindrop Foundation & Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first runnable Raindrop vertical slice: Rust/Axum service, ASTRYX React UI embedded in the release binary, layered configuration, SQLite migrations, secure first-run setup, local admin login, server-side sessions, Chinese/English UI, and baseline CI.

**Architecture:** A modular monolith owns HTTP, configuration, persistence, authentication, and static assets. Before setup, the router exposes only bootstrap/setup/health and the SPA; after setup it connects to SQLite, migrates users/roles/sessions, and exposes local authentication. The frontend is split by feature and uses ASTRYX 0.1.6 components directly.

**Tech Stack:** Rust 1.94+, Axum 0.8, Tokio 1, SeaORM 1.1.19, SQLite, Argon2id, React 19.2.7, TypeScript 7.0.2, Vite 8.1.4, ASTRYX 0.1.6, Lingui 6.5.0, Vitest 4.1.10.

## Global Constraints

- Rust edition 2024 and MSRV 1.94.
- Pin `@astryxdesign/core`, `theme-neutral`, and CLI to exactly 0.1.6; never use 0.1.5.
- External input is validated at HTTP/config boundaries; errors never echo secrets.
- Browser auth uses an HttpOnly SameSite=Lax cookie; state changes require CSRF.
- Setup completion requires the one-time terminal token and becomes unavailable after the first user exists.
- Frontend controls use ASTRYX; business CSS is limited to domain layout and article typography.
- Frontend code is feature-oriented; TypeScript/TSX files default to at most about 250 lines.
- Mobile uses the same feature state/API with ASTRYX `MobileNav`; 390×844 and 360×800 flows have 44px touch targets and safe-area handling.
- Use test-first steps and commit after every independently passing task.

---

### Task 1: Repository and Rust HTTP skeleton

**Files:**

- Create: `.gitignore`
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/app.rs`
- Test: `tests/health_api.rs`

**Interfaces:**

- Produces: `pub fn build_router(state: AppState) -> axum::Router`
- Produces: `#[derive(Clone)] pub struct AppState { pub version: &'static str }`
- Produces: `GET /api/v1/health/live -> {"status":"ok"}`

- [ ] **Step 1: Write the failing health test**

```rust
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use raindrop::{app::{build_router, AppState}};
use tower::ServiceExt;

#[tokio::test]
async fn live_health_returns_ok() {
    let response = build_router(AppState::for_test())
        .oneshot(Request::builder().uri("/api/v1/health/live").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], br#"{"status":"ok"}"#);
}
```

- [ ] **Step 2: Run the test and confirm the crate is missing**

Run: `cargo test --test health_api`

Expected: failure because `Cargo.toml` and the `raindrop` library do not exist.

- [ ] **Step 3: Add the Rust package and minimal router**

Use exact stable dependency ranges in `Cargo.toml`; enable Axum JSON/macros, Tokio macros/rt-multi-thread/signal, and Tower util. `src/app.rs` returns JSON from a dedicated `health` function. `src/main.rs` binds `RAINDROP_BIND` or `0.0.0.0:8080`, installs tracing, and exits cleanly on Ctrl-C/SIGTERM.

Core implementation shape:

```rust
#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
}

impl AppState {
    pub fn for_test() -> Self {
        Self { version: env!("CARGO_PKG_VERSION") }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health/live", get(|| async { Json(json!({"status": "ok"})) }))
        .with_state(state)
}
```

- [ ] **Step 4: Add repository ignores**

Ignore `.worktrees/`, `target/`, `.env*` except `.env.example`, `data/`, `web/node_modules/`, `web/dist/`, Playwright output, and editor/OS files. Do not ignore lockfiles.

- [ ] **Step 5: Verify**

Run: `cargo fmt --check && cargo test --test health_api`

Expected: one passing integration test.

- [ ] **Step 6: Commit**

```bash
git add .gitignore rust-toolchain.toml Cargo.toml Cargo.lock src tests/health_api.rs
git commit -m "feat: bootstrap rust service"
```

### Task 2: Layered configuration and bootstrap mode

**Files:**

- Create: `src/config/mod.rs`
- Create: `src/config/model.rs`
- Create: `src/config/loader.rs`
- Create: `src/config/redact.rs`
- Test: `tests/config_loading.rs`

**Interfaces:**

- Produces: `pub struct LoadedConfig { pub runtime: RuntimeConfig, pub mode: BootstrapMode, pub sources: ConfigSources }`
- Produces: `pub enum BootstrapMode { SetupRequired { token: SecretString }, Ready }`
- Produces: `pub fn load(args: &ConfigArgs, env: &impl EnvSource) -> Result<LoadedConfig, ConfigError>`

- [ ] **Step 1: Write precedence and setup-mode tests**

Cover these cases with a temporary data directory and a map-backed environment source:

```rust
#[test]
fn no_database_source_enters_setup_mode() {
    let data = tempfile::tempdir().unwrap();
    let args = ConfigArgs::for_test(data.path());
    let loaded = load(&args, &MapEnv::default()).unwrap();
    assert!(matches!(loaded.mode, BootstrapMode::SetupRequired { .. }));
}

#[test]
fn env_overrides_toml_without_echoing_secret() {
    let data = tempfile::tempdir().unwrap();
    std::fs::write(
        data.path().join("config.toml"),
        "database_url = 'sqlite://file-value.db?mode=rwc'\n",
    ).unwrap();
    let env = MapEnv::from([(
        "RAINDROP_DATABASE_URL",
        "postgres://reader:super-secret@db/raindrop",
    )]);
    let loaded = load(&ConfigArgs::for_test(data.path()), &env).unwrap();
    assert_eq!(loaded.runtime.database_kind(), DatabaseKind::Postgres);
    assert!(!format!("{loaded:?}").contains("super-secret"));
}

#[test]
fn invalid_bind_address_names_the_variable() {
    let data = tempfile::tempdir().unwrap();
    let env = MapEnv::from([("RAINDROP_BIND", "not-an-address")]);
    let error = load(&ConfigArgs::for_test(data.path()), &env).unwrap_err();
    assert!(error.to_string().contains("RAINDROP_BIND"));
    assert!(!error.to_string().contains("database"));
}
```

- [ ] **Step 2: Run the tests and verify missing module failures**

Run: `cargo test --test config_loading`

Expected: compile failure because `raindrop::config` is missing.

- [ ] **Step 3: Implement typed config**

Use explicit structs instead of a stringly-typed map:

```rust
pub struct RuntimeConfig {
    pub bind: SocketAddr,
    pub public_url: Option<Url>,
    pub data_dir: PathBuf,
    pub database_url: Option<SecretString>,
    pub session_secret: Option<SecretString>,
    pub bootstrap_admin: Option<BootstrapAdmin>,
}
```

Load defaults, then TOML, then recognized `RAINDROP_*` variables. `RAINDROP_DATABASE_URL` selects managed configuration. Generate a 32-byte setup token with the OS RNG only when neither a config file nor database environment variable exists. Implement custom `Debug`/display helpers that show `<redacted>` for URL passwords, session secrets, setup token, and bootstrap password.

- [ ] **Step 4: Verify**

Run: `cargo test --test config_loading && cargo clippy --all-targets -- -D warnings`

Expected: all config tests pass and clippy is clean.

- [ ] **Step 5: Commit**

```bash
git add src/config src/lib.rs tests/config_loading.rs Cargo.toml Cargo.lock
git commit -m "feat: add layered configuration"
```

### Task 3: SQLite connection and identity migrations

**Files:**

- Create: `src/db/mod.rs`
- Create: `src/db/connect.rs`
- Create: `src/db/migration.rs`
- Create: `src/db/entities.rs`
- Test: `tests/database_migrations.rs`

**Interfaces:**

- Produces: `pub async fn connect(config: &DatabaseConfig) -> Result<DatabaseConnection, DbError>`
- Produces: `pub async fn migrate(db: &DatabaseConnection) -> Result<(), DbError>`
- Produces entities for `users`, `user_roles`, and `sessions`.

- [ ] **Step 1: Write the migration contract test**

Create a temporary SQLite file, connect, run migrations twice, and assert:

- `PRAGMA journal_mode` is `wal` for file databases.
- foreign keys are enabled.
- duplicate normalized usernames fail.
- duplicate `(user_id, role)` fails.
- deleting a user cascades roles and sessions.

- [ ] **Step 2: Run the test and confirm failure**

Run: `cargo test --test database_migrations`

Expected: compile failure because the database module does not exist.

- [ ] **Step 3: Implement connection policy**

Use SeaORM 1.1.19 with `sqlx-sqlite`, `sqlx-postgres`, `sqlx-mysql`, `runtime-tokio-rustls`, `with-time`, and `with-uuid`. SQLite connect options use one writer by default, foreign keys, a 5-second busy timeout, and WAL for file URLs. Keep backend detection in `connect.rs` so later PostgreSQL/MySQL contract tests reuse the same API.

- [ ] **Step 4: Implement portable migrations**

Use UUID strings/binary-compatible columns, UTC timestamps, and named unique/foreign-key indexes. `sessions` stores `token_hash` and `csrf_hash`, never raw tokens. Every migration has an idempotent down path for test databases.

- [ ] **Step 5: Verify**

Run: `cargo test --test database_migrations`

Expected: the SQLite migration contract passes twice on a fresh temporary database.

- [ ] **Step 6: Commit**

```bash
git add src/db src/lib.rs tests/database_migrations.rs Cargo.toml Cargo.lock
git commit -m "feat: add identity database schema"
```

### Task 4: User repository and Argon2id authentication service

**Files:**

- Create: `src/auth/mod.rs`
- Create: `src/auth/password.rs`
- Create: `src/auth/users.rs`
- Create: `src/auth/model.rs`
- Test: `tests/local_auth.rs`

**Interfaces:**

- Produces: `pub async fn create_admin(db, input) -> Result<User, CreateUserError>`
- Produces: `pub async fn authenticate(db, login, password) -> Result<User, AuthenticateError>`
- Produces: `pub fn normalize_username(value: &str) -> Result<String, UsernameError>`

- [ ] **Step 1: Write local-auth tests**

Test successful admin creation/login, duplicate case-insensitive username rejection, wrong-password generic failure, disabled-user rejection, and password hash verification. Assert serialized users never include `password_hash`.

- [ ] **Step 2: Run the tests and confirm missing service failures**

Run: `cargo test --test local_auth`

- [ ] **Step 3: Implement Argon2id and repository operations**

Use Argon2id with explicit memory/time/parallelism minimums and random salts. Accept usernames 3–64 Unicode scalar values after trim + lowercase login normalization; preserve the display username separately. Create the user and `ADMIN`/`USER` roles in one transaction. Map unique conflicts to `UsernameTaken` without exposing SQL.

- [ ] **Step 4: Verify**

Run: `cargo test --test local_auth`

Expected: all local authentication tests pass; a password is never present in a debug snapshot.

- [ ] **Step 5: Commit**

```bash
git add src/auth src/lib.rs tests/local_auth.rs Cargo.toml Cargo.lock
git commit -m "feat: add local admin authentication"
```

### Task 5: Hashed sessions, cookies, and CSRF

**Files:**

- Create: `src/auth/sessions.rs`
- Create: `src/auth/cookies.rs`
- Create: `src/auth/extractor.rs`
- Modify: `src/auth/mod.rs`
- Test: `tests/session_security.rs`

**Interfaces:**

- Produces: `SessionService::create(user_id) -> CreatedSession { cookie_token, csrf_token, expires_at }`
- Produces: `CurrentUser` Axum extractor.
- Produces: `CsrfGuard` Axum extractor for state-changing cookie-authenticated requests.

- [ ] **Step 1: Write session security tests**

Assert raw tokens are absent from the database, cookie flags are correct, expired/revoked sessions fail, CSRF mismatch returns 403, and `last_seen_at` is not updated more than once per 15 minutes.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test --test session_security`

- [ ] **Step 3: Implement sessions**

Generate a random 256-bit session token and derive a distinct 256-bit CSRF token with BLAKE3 domain separation; store only their hashes. The one-way derivation lets `/auth/session` recover the same CSRF value after reload without storing it raw or invalidating other browser tabs. Cookie name is `raindrop_session`; set HttpOnly, SameSite=Lax, Path=/, and Secure when `public_url` is HTTPS. `CurrentUser` joins user roles and rejects disabled accounts. `CsrfGuard` requires `X-CSRF-Token`, compares hashes in constant time, and verifies Origin/Host when Origin is present.

- [ ] **Step 4: Verify**

Run: `cargo test --test session_security`

- [ ] **Step 5: Commit**

```bash
git add src/auth tests/session_security.rs Cargo.toml Cargo.lock
git commit -m "feat: secure browser sessions"
```

### Task 6: Bootstrap, setup, and auth HTTP APIs

**Files:**

- Create: `src/api/mod.rs`
- Create: `src/api/error.rs`
- Create: `src/setup/mod.rs`
- Create: `src/setup/service.rs`
- Create: `src/api/routes.rs`
- Modify: `src/app.rs`
- Test: `tests/setup_auth_api.rs`

**Interfaces:**

- Produces the `/api/v1/bootstrap`, `/setup/database-check`, `/setup/complete`, `/auth/login`, `/auth/logout`, and `/auth/session` contracts from the design spec.
- Produces uniform `ApiErrorBody` with `code`, `message`, optional `fields`, and `requestId`.

- [ ] **Step 1: Write setup/auth API tests**

Cover:

- bootstrap reports `SETUP_REQUIRED` without leaking the setup token.
- setup complete without `X-Setup-Token` returns 401.
- bad database URL returns 422 with a field error and no password.
- successful setup atomically writes `config.toml` with Unix mode 0600 and creates one admin.
- a second setup call returns 409.
- login sets the session cookie; session returns the CSRF token; logout revokes it.

- [ ] **Step 2: Run and confirm missing routes**

Run: `cargo test --test setup_auth_api`

- [ ] **Step 3: Implement setup state machine**

`SetupService` owns the one-time token and a mutex that serializes completion. It validates and tests the database before writing config, writes a temporary file in the same directory, syncs, chmods 0600 on Unix, then renames atomically. It connects/migrates and creates the initial admin before reporting success. If any step fails, it removes the temporary file and leaves setup available with the same token.

- [ ] **Step 4: Implement API errors and auth routes**

All validation failures use 422, authentication 401, authorization/CSRF 403, conflicts 409, and internal errors 500 with a request ID. Never put `source` error text into the response body. Apply stricter in-memory rate limits to setup/login endpoints in this stage; database/distributed rate limiting can replace it later without changing route contracts.

- [ ] **Step 5: Verify**

Run: `cargo test --test setup_auth_api`

- [ ] **Step 6: Commit**

```bash
git add src/api src/setup src/app.rs tests/setup_auth_api.rs Cargo.toml Cargo.lock
git commit -m "feat: add setup and login APIs"
```

### Task 7: ASTRYX React foundation and feature modules

**Files:**

- Create: `web/package.json`
- Create: `web/package-lock.json`
- Create: `web/index.html`
- Create: `web/vite.config.ts`
- Create: `web/tsconfig.json`
- Create: `web/lingui.config.ts`
- Create: `web/src/main.tsx`
- Create: `web/src/app/App.tsx`
- Create: `web/src/app/Providers.tsx`
- Create: `web/src/shared/theme/raindrop.css`
- Create: `web/src/shared/i18n/i18n.ts`
- Test: `web/src/app/App.test.tsx`

**Interfaces:**

- Produces a small `App.tsx` that only chooses setup, login, or ready routes from bootstrap state.
- Produces root ASTRYX `Theme`, `LinkProvider`, and `LayerProvider` integration.

- [ ] **Step 1: Create package metadata with exact versions**

Pin React 19.2.7, Vite 8.1.4, TypeScript 7.0.2, ASTRYX core/theme-neutral/CLI 0.1.6, StyleX 0.19.x, Lingui 6.5.0, React Router 7.18.1, Vitest 4.1.10, and Testing Library. Define `dev`, `build`, `typecheck`, `test:ci`, `lint`, and `astryx:check` scripts.

- [ ] **Step 2: Install with scripts disabled and inspect**

Run: `npm --prefix web install --ignore-scripts`

Inspect `web/package-lock.json` and any packages declaring install scripts before allowing a clean `npm ci`.

- [ ] **Step 3: Write the failing app routing test**

Mock `/api/v1/bootstrap` and assert `SETUP_REQUIRED` renders the setup heading in both locale catalogs, while `READY` without a session renders login.

- [ ] **Step 4: Implement providers and small app shell**

Import CSS in this order: ASTRYX reset, ASTRYX base, neutral theme, then `raindrop.css`. Adapt React Router through `LinkProvider`. The Raindrop CSS file may set theme tokens for warm paper/ink blue and serif reading typography, but must not target ASTRYX internal descendant selectors.

- [ ] **Step 5: Verify ASTRYX package and production build**

Run:

```bash
node web/node_modules/@astryxdesign/core/docs.mjs AppShell
node web/node_modules/@astryxdesign/core/docs.mjs FormLayout
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
```

Expected: docs resolve, tests pass, and the production bundle imports `react/jsx-runtime`, not `react/jsx-dev-runtime`.

- [ ] **Step 6: Commit**

```bash
git add web
git commit -m "feat: add astryx web foundation"
```

### Task 8: Setup and local-login frontend slices

**Files:**

- Create: `web/src/features/setup/api.ts`
- Create: `web/src/features/setup/model.ts`
- Create: `web/src/features/setup/SetupPage.tsx`
- Create: `web/src/features/setup/DatabaseStep.tsx`
- Create: `web/src/features/setup/AdminStep.tsx`
- Create: `web/src/features/auth/api.ts`
- Create: `web/src/features/auth/LoginPage.tsx`
- Create: `web/src/features/auth/session.ts`
- Create: `web/src/features/reader/ReadyPage.tsx`
- Create: `web/src/features/reader/ReadyMobilePage.tsx`
- Create: `web/src/shared/responsive/useViewportMode.ts`
- Create: `web/src/shared/api/client.ts`
- Test: adjacent `*.test.tsx` files for setup and login.

**Interfaces:**

- Consumes the setup/auth API from Task 6.
- Produces feature-local APIs and pages; no file exceeds about 250 lines.

- [ ] **Step 1: Write setup interaction tests**

Test setup token entry, SQLite default, PostgreSQL/MySQL URL selection, database check error banner, admin validation, completion, locale switch, and loading states. Assert controls come from ASTRYX imports rather than local generic wrappers.

- [ ] **Step 2: Implement setup pages from ASTRYX components**

Use `FormLayout`, `TextInput`, `Selector` or `RadioList`, `ProgressBar`, `Banner`, `Button`, `Section`, and `Code` for environment variable names. Keep step state in `model.ts`; each step component owns only its fields and callbacks. Do not use `TabList` as a fake stepper.

- [ ] **Step 3: Write and implement login/session tests**

Use `Center`, `Card`, `FormLayout`, `TextInput`, `Button`, and `Banner`. Store CSRF only in memory; never localStorage. `ReadyPage` uses `AppShell`, `Layout`, `EmptyState`, and a logout button, proving authenticated navigation without implementing RSS early.

At widths below 720px, render `ReadyMobilePage` through the same session model. It uses `AppShell + MobileNav`, a single content task, safe-area padding, and an explicit menu/logout route. Do not render compressed desktop side panels.

- [ ] **Step 4: Verify module size and UI tests**

Run:

```bash
npm --prefix web run typecheck
npm --prefix web run test:ci
find web/src -name '*.ts' -o -name '*.tsx' | xargs wc -l | sort -nr | head -20
```

Expected: tests pass and no feature file exceeds 250 lines without an explicit documented reason.

Run Testing Library with `matchMedia`/viewport mocks for 390px and 1280px; assert both modes expose the same authenticated actions and no mobile action depends on hover.

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "feat: add setup and login interface"
```

### Task 9: Embedded production UI and end-to-end smoke

**Files:**

- Create: `build.rs`
- Create: `src/web/mod.rs`
- Create: `src/web/assets.rs`
- Modify: `src/app.rs`
- Modify: `Cargo.toml`
- Test: `tests/embedded_web.rs`
- Test: `web/e2e/setup-login.spec.ts`
- Test: `web/e2e/mobile-foundation.spec.ts`

**Interfaces:**

- Produces immutable hashed-asset responses and SPA fallback from `web/dist` in release builds.
- Produces a debug response that points developers to the Vite dev server without requiring `web/dist` for Rust unit tests.

- [ ] **Step 1: Write embedded asset tests**

Build the web bundle, then assert `/` returns HTML, an asset returns the correct MIME type and immutable cache header, unknown non-API routes return SPA HTML, and unknown `/api/*` routes remain JSON 404.

- [ ] **Step 2: Implement build and asset serving**

`build.rs` emits `rerun-if-changed` for web sources and fails release builds with a precise instruction if `web/dist/index.html` is absent. `rust-embed` is compiled only for non-debug builds; debug mode returns a small development page. Add CSP, `X-Content-Type-Options: nosniff`, `Referrer-Policy`, and frame denial headers.

- [ ] **Step 3: Add Playwright setup/login smoke**

Run the Rust test server with a temporary data directory and the production web bundle. Complete setup using the emitted token, log in, verify the ready shell, log out, and confirm setup cannot reopen.

Add mobile projects at 390×844 and 360×800. Verify setup fields and primary action fit without horizontal scrolling, every actionable control has at least a 44px bounding box, `MobileNav` is keyboard/touch reachable, and browser Back does not lose the authenticated shell state.

- [ ] **Step 4: Verify release binary**

Run:

```bash
npm --prefix web run build
cargo test --all-features
cargo build --release --locked
./target/release/raindrop --version
```

Expected: all tests pass; the binary reports version; no Node process is needed at runtime.

- [ ] **Step 5: Commit**

```bash
git add build.rs src/web src/app.rs Cargo.toml Cargo.lock tests web/e2e
git commit -m "feat: embed production web interface"
```

### Task 10: Baseline CI and foundation documentation

**Files:**

- Create: `.github/workflows/ci.yml`
- Create: `.env.example`
- Create: `README.md`
- Create: `docs/configuration.md`
- Modify: `tasks/todo.md`

**Interfaces:**

- Produces a reproducible CI gate for Rust, ASTRYX web, SQLite, and release embedding.
- Documents interactive and environment-managed first run without real secrets.

- [ ] **Step 1: Add CI workflow**

Jobs run `npm ci --ignore-scripts`, the reviewed ASTRYX production build, typecheck/tests, Rust fmt/clippy/tests, and `cargo build --release --locked`. Cache npm and Cargo data by lockfile. Upload no secret-bearing data directory.

- [ ] **Step 2: Document startup and security behavior**

README shows interactive SQLite setup and environment initialization. `docs/configuration.md` defines precedence, all stage-1 variables, setup token behavior, file permissions, reverse proxy HTTPS expectations, and SQLite network-filesystem warning.

- [ ] **Step 3: Mark the foundation checklist complete only from evidence**

Update each `tasks/todo.md` foundation checkbox only after its corresponding verification command passes.

- [ ] **Step 4: Run the full foundation gate**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
npm --prefix web ci --ignore-scripts
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cargo build --release --locked
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 5: Commit**

```bash
git add .github .env.example README.md docs/configuration.md tasks/todo.md
git commit -m "ci: verify foundation builds"
```

## Plan self-review

- Spec coverage: this plan covers stage 1 only and leaves all later stages explicitly tracked in `tasks/todo.md`; it does not claim RSS/OIDC/AI/plugin/MCP/OPML/release completion.
- Type consistency: `AppState`, `LoadedConfig`, `SetupService`, `SessionService`, and API paths are introduced before their consumers.
- Security: setup takeover, password storage, raw session tokens, CSRF, secret redaction, file permissions, and production dependency smoke have direct tests.
- Frontend: ASTRYX exact version, component mapping, production build regression, feature directories, and file-size rule are directly verified.
- Placeholder scan: no unresolved implementation decision is delegated to the task executor.
