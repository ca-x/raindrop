# Production AI Bundle and Runtime v1 Implementation Plan

> **Execution:** Inline main-Agent execution only. The user explicitly prohibited sub-agent development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Embed a signed official AI component into every binary and run the existing content worker under the real setup/startup/shutdown lifecycle.

**Architecture:** The build script deterministically builds, componentizes, signs, and emits immutable assets into `OUT_DIR`. Runtime code verifies those assets, synchronizes the bundled installation after setup readiness, composes provider/Wasm/content services, and supervises Content and Feed runtimes as one background unit.

**Tech Stack:** Rust 2024/MSRV 1.94, Cargo build scripts, `wit-component`, Ed25519/SHA-256 via `ring`, Wasmtime Component Model, Tokio, SeaORM, GitHub Actions, Docker BuildKit secrets.

## Global constraints

- Main Agent only; one bounded internal DDIA/security/contract review.
- No production seed in source, files, logs, artifacts, caches, image layers, or final binaries.
- Local/non-release builds are visibly development-signed; tag/Docker publication requires the protected official seed.
- No database transaction spans Wasm/provider work.
- No provider/plugin/content API, lifecycle dispatcher, MCP transport, or Reader AI UI in this slice.
- Exact `apply_patch` edits and exact staging; never `git add -A`.

---

### Task 1: Build, componentize, and sign the official guest

**Files:**

- Create: `build/official_ai.rs`
- Modify: `build.rs`
- Modify: `Cargo.toml`
- Modify: `rust-toolchain.toml`
- Test: `tests/embedded_official_ai_bundle.rs`

**Interfaces:**

- Produces generated component, canonical manifest, and 32-byte public key under `OUT_DIR`.
- Produces compile-time environment values `RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID` and `RAINDROP_OFFICIAL_PLUGIN_SIGNATURE_MODE`.

- [x] Add build dependencies for canonical JSON, base64url, Ed25519/SHA-256, componentization, Wasm inspection, and zeroization without changing runtime dependency versions.
- [x] Declare `wasm32-unknown-unknown` in `rust-toolchain.toml`.
- [x] Add a failing embedded-bundle test that references the generated assets and exact mode/key ID contract.
- [x] Implement the nested locked guest release build in an isolated `OUT_DIR` target directory and remove inherited target/rustflag contamination.
- [x] Componentize twice, require byte identity, validate the component, and reject any `wasi:` import.
- [x] Finalize the manifest template, canonicalize it, sign the frozen v1 frame, write only component/manifest/public key, and zeroize the seed.
- [x] Require the official base64url 32-byte seed when `RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE=1`; otherwise use the public development derivation and label it.
- [x] Run the embedded-bundle test and a forced-official missing-seed negative build probe.

### Task 2: Verify and expose the immutable embedded bundle

**Files:**

- Create: `src/plugins/bundled.rs`
- Modify: `src/plugins/mod.rs`
- Modify: `tests/embedded_official_ai_bundle.rs`
- Modify: `web/package.json`

**Interfaces:**

- Produces `EmbeddedOfficialAiPlugin::load`, `bundle`, `component`, `signature_mode`, and `compile`.
- Produces `EmbeddedSignatureMode::{Development,Official}`.

- [x] Verify the embedded public key length and build-emitted key ID before calling `BundledOfficialPlugin::verify`.
- [x] Require exact plugin key/version/ABI/digest/signature identity and compile only through `CompiledPlugin::compile`.
- [x] Keep component, manifest, and public key sources private and immutable; expose no path or replacement hook.
- [x] Extend release embedding tests so `npm --prefix web run test:e2e` exercises the real embedded plugin bundle.
- [x] Run debug and release embedded-bundle tests.

### Task 3: Compose the setup-aware production Content runtime

**Files:**

- Create: `src/content/worker/production.rs`
- Modify: `src/content/worker/runtime.rs`
- Modify: `src/content/worker/mod.rs`
- Create: `tests/content_runtime_production.rs`

**Interfaces:**

- Produces `ProductionContentRuntime::new` and `run`.
- Reuses one `ContentRuntimeHandle` across setup waiting and active lanes.
- Adds a crate-visible controlled constructor for `ContentRuntime` plus `ContentRuntimeHandle::inert`.

- [x] Add tests for pre-ready inert shutdown, setup transition without restart, installation synchronization, absent-keyring no-claim behavior, valid-keyring lane startup/shutdown, and structural sync failure.
- [x] Verify and compile the embedded component before any installation write.
- [x] Wait on setup readiness with shutdown-aware one-second polling.
- [x] Synchronize the bundled installation; retry only database/revision failures and fail closed on corrupt/identity failures.
- [x] Keep an empty provider keyring inert without generating fallback material.
- [x] Compose `ProviderRepository`, `HttpsProviderTransport`, `ProviderClient`, `ProviderAiBroker`, `OfficialAiProcessor`, and the existing eight-lane runtime for a valid keyring.
- [x] Run focused production runtime tests with one test thread.

### Task 4: Supervise Feed and Content runtimes together

**Files:**

- Create: `src/background.rs`
- Modify: `src/lib.rs`
- Modify: `src/app.rs`
- Modify: `src/main.rs`
- Test: `src/main.rs` unit tests

**Interfaces:**

- Produces `BackgroundRuntime::production`, `BackgroundRuntime::run`, and `BackgroundRuntimeHandle::{feed,content,shutdown}`.
- Extends `AppState::with_runtimes` while preserving `with_feed_runtime` compatibility.

- [x] Add supervisor tests proving normal shutdown stops both children before server drain.
- [x] Add tests proving unexpected Feed or Content completion stops and joins its sibling and then stops the server.
- [x] Build both production runtimes from the loaded setup, retention config, and provider-secret entries.
- [x] Store both handles in `AppState`; keep existing Feed post-commit notification behavior unchanged.
- [x] Replace the single-runtime coordinator input with the background group task and preserve redacted typed error chains.
- [x] Run binary unit tests and existing Feed runtime/API tests.

### Task 5: Enforce official signing in release and Docker delivery

**Files:**

- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release-binaries.yml`
- Modify: `.github/workflows/docker.yml`
- Modify: `Dockerfile`
- Modify: `web/scripts/verify-release-contracts.mjs`

**Interfaces:**

- Consumes GitHub secret `RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED`.
- Requires key ID `raindrop-release-2026` for tagged binary and Docker builds.

- [x] Install the wasm target in every CI/release lane that compiles the root package.
- [x] Set the official signing requirement and secret environment for all five tagged binary targets.
- [x] Pass the seed to Docker only through a BuildKit secret mount and keep the requirement as a non-secret build argument.
- [x] Keep CI release/container smoke on development mode and assert that tagged workflows cannot silently fall back.
- [x] Extend the release contract verifier for target, secret, key ID, requirement, and no-build-arg seed rules.
- [x] Run `npm --prefix web run check:release-contracts` and full local release E2E; the host has no Docker CLI, so the real container runtime smoke is delegated to the corresponding pushed CI run.

### Task 6: Document, verify once, commit, and push

**Files:**

- Modify: `.env.example`
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/ai-providers.md`
- Modify: `tasks/plan.md`, `tasks/todo.md`, and this plan

- [x] Document development versus official embedded signature modes and the GitHub secret provisioning/rotation contract without showing key material.
- [x] Run all local commands from the design specification plus existing official component, registry, provider, worker, and release E2E gates; the pushed CI run owns the unavailable local container gate.
- [x] Perform one bounded DDIA/security/contract review and fix only confirmed findings.
- [x] Run source confinement, real secret-pattern scan, `git diff --check`, and verify no seed is written anywhere.
- [x] Provision or confirm the GitHub Actions signing secret through a non-echoing command before claiming tag workflows are usable.
- [ ] Stage exact files, inspect the staged diff, commit, push `main`, and monitor the corresponding CI run.

## Self-review

- Spec coverage: build/signing, immutable embedding, setup transition, installation sync, optional keyring, real provider composition, joint supervision, release enforcement, and explicit exclusions each map to one task.
- Type consistency: the embedded bundle produces the exact `BundledOfficialPlugin`/`CompiledPlugin` consumed by `OfficialAiProcessor`; the production wrapper reuses `ContentRuntimeHandle`; the background group exposes the existing Feed handle to `AppState`.
- DDIA: database synchronization is short/idempotent; external execution stays transaction-free; startup polling is liveness-only; shutdown and lease recovery preserve safety.
- Security: official publication fails without a protected seed, Docker uses a secret mount, development mode is explicit, and no user-controlled plugin source is introduced.
