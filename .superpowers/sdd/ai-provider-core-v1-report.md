# AI Provider Core v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`
Status: VERIFIED

## Delivered scope

- Optional, validated `RAINDROP_PROVIDER_SECRET_KEYS` / TOML keyring configuration with environment replacement semantics.
- Dedicated ring AES-256-GCM envelope `rdsec1.<key-id>.<nonce>.<ciphertext+tag>`, active-first rotation, provider ID/kind AAD binding, canonical base64url validation, and redacted errors.
- Portable `ai_providers` schema/entity/repository contracts for SQLite, PostgreSQL, and MySQL.
- Instance/user provider scope, active-user checks, immutable provider kind, enabled binding load, normalized capability/quota/cost policy, and revision CAS updates.
- Four protocol families: Anthropic Messages-compatible, OpenAI Responses, OpenAI Chat Completions-compatible, and Google Gemini.
- HTTPS-only provider endpoint normalization with safe path-prefix joining.
- DNS-pinned, public-only, no-proxy, no-redirect provider POST transport with peer verification, staged deadlines, bounded body/decompression, and `Retry-After` parsing.
- One redacted `ProviderClient` composing binding → adapter → transport → decoder with stable call error kinds.

This slice is internal core only. It does not expose provider administration HTTP/UI, content jobs/artifacts, summary/translation operations, the official Wasm plugin, MCP, streaming, or live provider credentials.

## Security and data evidence

- Provider credentials are never derived from the session secret and are never stored as plaintext.
- The first keyring entry encrypts new credentials; retained entries decrypt old envelopes. Duplicate IDs/material and padded/non-canonical base64 fail closed.
- AAD binds ciphertext to both provider ID and immutable provider kind; wrong ID/kind, tampering, and unknown key ID fail decryption.
- `ai_providers` is the authoritative record. Policy fields are normalized columns rather than provider-specific JSON.
- The owner foreign key cascades on user deletion; `idx_ai_providers_owner_enabled` supports scoped enabled lookup and `idx_ai_providers_kind` supports kind inventory across all three databases.
- Repository reads and writes use explicit scope; binding decryption occurs only after scope, active-user, enabled, kind, endpoint, capability and policy validation.
- Any denied DNS answer rejects the entire set. Approved sockets are pinned into reqwest and the connected peer must match.
- Provider POSTs do not use proxies or redirects. Non-2xx bodies are not polled. Compressed and decoded response limits are 2 MiB with a 100x expansion ceiling.
- Metadata, binding and provider errors exclude full endpoint/path, model, credential, request, response, prompt, schema and model output. `ProviderEndpoint` diagnostics expose only scheme/host/port/path-segment count. reqwest sources have their URL removed.
- Model output remains untrusted structured data and must pass the existing canonical schema boundary.

## Focused deterministic verification

| Contract | Evidence |
| --- | --- |
| Configuration | 13 tests, including env-over-file replacement, malformed sentinel redaction and optional configuration |
| Secret keyring | 6 tests covering envelope, nonce, non-determinism, rotation, AAD mismatch, tampering, bounds, canonical encoding and redaction |
| Provider storage/model/repository | 7 tests; SQLite local execution plus PostgreSQL/MySQL service execution in CI |
| Provider adapters | 13 tests covering four request fixtures, four response fixtures, status/error normalization, bounds, schema rejection and redaction |
| HTTPS transport unit contracts | 20 tests covering DNS/address policy, pinning, peer/redirect/header controls, exact staged deadlines, `Retry-After`, body limits/encodings and debug redaction |
| Provider client integration | 6 tests covering four full protocol compositions, mismatch/policy rejection, transport/adapter error mapping, retry propagation, corrupt/disabled binding barriers and source boundaries |

## Commits

- `140bd6b docs: plan ai provider core`
- `a6fcf87 feat: encrypt ai provider credentials`
- `4a934e5 feat: persist ai provider records`
- `0e1b8d6 feat: define ai provider core model`
- `9347a18 feat: load scoped ai provider bindings`
- `1eedb75 feat: execute ai providers through pinned https`
- `8bbe410 feat: compose ai provider execution`

## Dependency and lockfile delta

- Added direct exact `ring = "=0.17.14"`; the same version was already present transitively through rustls.
- The root `Cargo.lock` package dependency list gained only the direct `ring` edge; no second ring version was introduced.
- Transport reuses already committed `reqwest`, `rustls`, `hickory-resolver`, `async-compression`, `http`, `httpdate`, `url`, `secrecy`, `zeroize`, `base64` and `blake3` dependencies.

## CI evidence

- `29649500765`: keyring/config commit matrix passed.
- `29650568581`: provider model/repository matrix passed, including SQLite, PostgreSQL and MySQL provider storage contracts.
- `29651448590`: HTTPS transport matrix passed, including Rust, current stable, Windows compile, Web, supply-chain, release E2E and non-root container jobs.
- `29652073241`: ProviderClient composition commit passed the complete matrix.
- `29652502840`: final verification commit passed all seven jobs:
  - Rust foundation, including formatting, Clippy, SQLite/PostgreSQL/MySQL provider storage and the full Rust suite;
  - Rust current-stable compatibility;
  - Rust Windows durable replacement compile;
  - ASTRYX Web typecheck, Vitest and production build;
  - supply-chain audit and registry signature verification;
  - release embedding and E2E;
  - non-root container build and live health.

The final run emitted the existing GitHub Actions annotation that pinned Docker actions still target the deprecated Node.js 20 runtime and are being forced onto Node.js 24. It did not fail the matrix and is separate follow-up work.

## Existing advisories

- `cargo audit` reports `RUSTSEC-2023-0071` for `rsa 0.9.10`, inherited through `sqlx-mysql 0.8.6`. It is a medium-severity Marvin timing attack in RSA private-key operations and has no fixed release. This dependency is not introduced by AI Provider Core. The SQLx MySQL path imports `RsaPublicKey` and OAEP only to encrypt an authentication response; Raindrop does not perform the vulnerable RSA private-key operation, so the advisory is not reachable in this runtime. It remains tracked until sqlx/rsa provides a fix; MySQL should still use TLS to avoid authentication downgrade and credential exposure.
- `proc-macro-error2 v2.0.1`, inherited through SeaORM macros, is reported as unmaintained by `RUSTSEC-2026-0173` and continues to emit the existing future-incompatibility warning. It is a build-time proc macro dependency; current builds/tests pass.
- `cargo audit --ignore RUSTSEC-2023-0071` exits successfully with only the recorded unmaintained warning, confirming no second untriaged vulnerability in the 404-crate lockfile graph.

## Final local gates

| Command/check | Result |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --locked --test ai_provider_secret -- --nocapture` | PASS — 6 tests |
| `cargo test --locked --test ai_provider_storage sqlite -- --nocapture --test-threads=1` | PASS — focused SQLite contract; PostgreSQL/MySQL URLs were not configured locally |
| `cargo test --locked --test ai_provider_transport -- --nocapture` | PASS — 6 client/integration tests |
| `cargo test --locked --test ai_provider_adapters -- --nocapture` | PASS — 13 tests |
| `cargo test --locked --all-features` | PASS — 508 passed, 0 failed, 1 ignored opt-in live IT之家 RSS smoke |
| provider transport library tests | PASS — 20 tests inside the all-features suite |
| `cargo audit --file Cargo.lock` | EXPECTED NON-ZERO — only tracked `RUSTSEC-2023-0071`; one unmaintained warning |
| `cargo audit --file Cargo.lock --ignore RUSTSEC-2023-0071` | PASS — no additional vulnerability; one recorded warning |
| secret/source-boundary scans | PASS — production secret exposure confined to config/keyring/repository/adapter/transport; provider wire markers confined to adapters |
| `git diff --check` | PASS |

Local PostgreSQL/MySQL destructive contract URLs were absent. Cross-database evidence comes from CI service databases in run `29650568581`, where all three provider storage steps passed.

## Explicitly remaining

- Provider administration API/UI and authorization service boundaries.
- Content job/artifact schema, scheduler, quota/cost reservation, retries, cache and invalidation.
- User-visible summary and translation operations.
- Official signed `raindrop.ai-content` Wasm Component, versioned WIT/manifest and capability host.
- Reader sidecar and artifact UI.
- MCP client broker and Raindrop MCP server.
- Credentialed live provider contract probe before user-visible provider configuration is enabled.

All AI/plugin/MCP todo items remain unchecked until those user-visible and runtime boundaries are implemented.
