# AI Provider Core v1 Implementation Plan

> **For agentic workers:** Execute inline in the main Agent only with `executing-plans`. Do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist encrypted AI provider bindings and execute the existing four structured-generation adapters through one SSRF-safe HTTPS transport.

**Architecture:** `ai_providers` is the authoritative cross-database record. `src/content/provider` owns typed metadata, an AES-256-GCM keyring, scoped repository, endpoint policy, pinned transport, and client composition; provider-specific JSON remains inside the four existing adapter files. Storage, adapter encoding, network execution, and adapter decoding are sequential trust boundaries with stable redacted errors.

**Tech Stack:** Rust 1.94.0 edition 2024, SeaORM/sea-orm-migration, ring 0.17.14 AES-256-GCM, reqwest/rustls, hickory-resolver, async-compression, secrecy, zeroize, existing Cargo lockfile.

## Global constraints

- Follow `docs/superpowers/specs/2026-07-18-ai-provider-core-v1-design.md` exactly.
- Main Agent only; do not use subagents.
- Do not modify `.superpowers/research/` or root `node_modules/`.
- Do not add provider administration API/UI, content jobs/artifacts, Wasmtime, lifecycle dispatch, MCP, or streaming generation.
- Do not send a real provider request or commit a real key/credential.
- Provider kind, endpoint, secret, model, policy, and scope validation occurs before network execution.
- Every error and `Debug` path must remain free of key material, plaintext/ciphertext credential, endpoint, prompt, schema, request/response body, and model output.
- Every implementation commit is formatted, focused-tested, pushed, and monitored only for concrete CI failure. The final verification commit runs the complete gates and all three database backends.

---

### Task 1: Dedicated rotatable provider-secret keyring

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/config/model.rs`
- Modify: `src/config/loader.rs`
- Modify: `tests/config_loading.rs`
- Modify: `src/content/provider/types.rs`
- Create: `src/content/provider/secret.rs`
- Modify: `src/content/provider/mod.rs`
- Create: `tests/ai_provider_secret.rs`

**Interfaces:**

- Produces `ProviderKind::as_storage()` plus `ProviderSecretKeyring::from_entries(&[SecretString])`, `encrypt(provider_id, kind, credential)`, and `decrypt(provider_id, kind, envelope)`.
- Produces optional `RuntimeConfig::provider_secret_keys() -> &[SecretString]` containing validated `key-id:base64url` entries in active-first order.
- Freezes the four existing provider kinds to their final uppercase storage values before the first ciphertext fixture is written.

- [x] Add `ring = "=0.17.14"` as a direct dependency and refresh only the root lockfile dependency list.
- [x] Write failing config tests named `provider_secret_key_environment_replaces_toml`, `provider_secret_key_entries_are_validated_without_echo`, and `provider_secret_key_configuration_is_optional`. Use deterministic 32-byte URL-safe base64 test keys and assert malformed sentinel values never appear in the full error chain.
- [x] Write failing secret tests covering exact envelope prefix, nonce length, non-deterministic ciphertext, active/previous key rotation, wrong provider ID, wrong kind, unknown key ID, tampering, empty/8,193-byte plaintext, malformed base64, duplicate key IDs/material, and redacted formatting.
- [x] Implement config grammar `RAINDROP_PROVIDER_SECRET_KEYS=id:key,id:key` and TOML `provider_secret_keys = ["id:key"]`; environment replaces the complete file list. Keep each raw entry in `SecretString` and validate ID/base64/32-byte length without preserving decoded bytes.
- [x] Add `ProviderKind::as_storage` with exact values `ANTHROPIC_MESSAGES`, `OPENAI_RESPONSES`, `OPENAI_CHAT_COMPLETIONS`, and `GOOGLE_GEMINI`; use it in every AAD byte sequence.
- [x] Implement `ProviderSecretKeyring` with ring `AES_256_GCM`, `SystemRandom`, 12-byte nonce, envelope `rdsec1.<id>.<nonce>.<ciphertext+tag>`, and AAD `raindrop.ai-provider-secret.v1\0<id>\0<kind>`. Keyring `Debug` prints only active ID and key count.
- [x] Use `base64::engine::general_purpose::URL_SAFE_NO_PAD`; reject padded encodings and non-canonical re-encoding. Zeroize decoded key buffers and temporary plaintext buffers after constructing `SecretString`.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test config_loading provider_secret -- --nocapture
cargo test --locked --test ai_provider_secret -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Explicitly stage the dependency, config, secret module, and two tests; scan the staged diff for test-only placeholders versus real secrets; commit and push `feat: encrypt ai provider credentials`.

### Task 2: Portable authoritative provider schema

**Files:**

- Create: `src/db/migration/ai_providers.rs`
- Modify: `src/db/migration.rs`
- Create: `src/db/entities/ai_provider.rs`
- Modify: `src/db/entities.rs`
- Create: `tests/ai_provider_storage.rs`
- Modify: `tests/support/database.rs`

**Interfaces:**

- Produces SeaORM `ai_provider::{Model, ActiveModel, Entity, Column}` for the exact specification columns.
- Produces integration entry points `sqlite_ai_provider_storage_contract`, `postgres_ai_provider_storage_contract`, and `mysql_ai_provider_storage_contract`.
- Does not yet expose repository operations; tests use the entity only to prove physical constraints and encrypted text round-trip.

- [x] Write failing SQLite schema assertions for the table, both indexes, owner FK cascade, operational timestamp type, boolean/integer/text round-trip, idempotent migrate, and rollback/up recreation.
- [x] Add PostgreSQL/MySQL variants using the existing optional test URL harness and `--test-threads=1`; make absent variables print one skip line and return, matching existing repository contracts.
- [x] Implement `CreateAiProviders` after user preferences in the migrator. Use string UUID IDs, nullable user FK with cascade, text endpoint/envelope, bounded string kind/model/name, booleans, signed database integers for validated policy values, revision, and `operational_timestamp` for both timestamps.
- [x] Add `idx_ai_providers_owner_enabled` and `idx_ai_providers_kind`; use stable explicit names and `if_not_exists`.
- [x] Add the entity in its own file and re-export it from `entities.rs`. Keep the ORM model private to persistence callers; never format it in tests because it contains ciphertext and endpoint.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_storage sqlite_ai_provider_storage_contract -- --nocapture --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Explicitly stage migration/entity/storage-test files; commit and push `feat: persist ai provider records`.

### Task 3: Provider model, immutable kind, endpoint, capability, and policy contracts

**Files:**

- Create: `src/content/provider/model.rs`
- Modify: `src/content/provider/types.rs`
- Modify: `src/content/provider/mod.rs`
- Modify: `tests/ai_provider_storage.rs`
- Modify: `tests/ai_provider_secret.rs`

**Interfaces:**

- Produces `ProviderKind::{from_storage, default_endpoint}` while preserving Task 1's frozen `as_storage` values.
- Produces `ProviderScope`, `ProviderEndpoint`, `ProviderCapabilities`, `ProviderPolicy`, `CreateProvider`, `UpdateProvider`, `ProviderMetadata`, `ProviderBinding`, `ProviderCoreError`, and exact validation/conversion helpers used by the repository and client.
- `ProviderEndpoint::join_adapter_path(&self, path: &str) -> Result<url::Url, ProviderCoreError>` is the only raw request URL construction entry.

- [x] Write failing tests for every four-kind storage/default mapping and unknown storage value.
- [x] Add endpoint tests for HTTPS defaults, path-prefix join, normalized trailing slash, length, userinfo/query/fragment, HTTP, public/private literal IP, literal/encoded dot segments, encoded slash/backslash, adapter `//`, query/fragment/backslash, and redacted `Debug`.
- [x] Add exact validation tests for display name/model, user UUID scope, v1 streaming rejection, concurrency/rate/token/cost lower and upper bounds, empty patch, maximum in-memory revision, and redacted metadata/binding/error formatting. Persisted negative/overflow/corrupt conversions remain in Task 4's repository contract.
- [x] Implement `from_storage` for Task 1's stable kind strings and the exact default endpoint mapping; prove unknown values fail closed and existing envelope fixtures remain unchanged.
- [x] Implement `ProviderEndpoint` with private `Url`, no `Display`/Serde, HTTPS-only authority, public literal classification via `AddressPolicy::public_only`, safe path-prefix normalization, and exact adapter path joining.
- [x] Implement typed capability/policy validation; Task 4 applies checked conversions to signed database integers. `supports_streaming = true` returns `UnsupportedCapability`.
- [x] Implement custom `Debug` for metadata/binding/errors. Endpoint/model/credential are `[REDACTED]`; metadata exposes no secret field.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_secret -- --nocapture
cargo test --locked --test ai_provider_storage provider_model -- --nocapture --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Explicitly stage focused model/type/test files; commit and push `feat: define ai provider core model`.

### Task 4: Scoped repository and encrypted binding load

**Files:**

- Create: `src/content/provider/repository.rs`
- Modify: `src/content/provider/mod.rs`
- Modify: `tests/ai_provider_storage.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**

- Produces `ProviderRepository::new(DatabaseConnection, ProviderSecretKeyring)`.
- Produces `create`, `get`, `list_for_user`, `update`, and `load_enabled_binding` with the exact signatures in the specification.
- Consumes Task 2 entity and Task 3 validated types; no HTTP/auth layer or generic ORM model escapes.

- [x] Write the full shared repository contract and run it first on SQLite. Cover instance/user creation, ciphertext-at-rest assertion through a direct test query, exact-scope visibility, Rust-sorted user listing independent of database collation, disabled binding, user-disabled create, cascade delete, immutable kind, unchanged ciphertext without credential patch, changed ciphertext with credential patch, revision conflict, unknown/corrupt kind, invalid policy, and tampered/unknown-key ciphertext.
- [x] Implement conversion from entity to metadata that validates every persisted field. Any invalid database fact returns `CorruptData` without formatting the entity.
- [x] Implement create as one short transaction: validate input, ensure active user for user scope, generate UUID, encrypt with ID/kind AAD, insert, and return metadata.
- [x] Implement exact scope predicates. Instance uses `owner_user_id IS NULL`; user uses exact UUID. Cross-scope ID and missing ID both return `NotFound`.
- [x] Implement update by loading the exact row, constructing and validating the complete prospective state, optionally encrypting a new credential, and issuing a revision-predicated `update_many`. Increment revision with checked arithmetic; zero rows returns `RevisionConflict`.
- [x] Implement binding load for `(instance OR requesting user) AND enabled`; decrypt only after the row passes scope, kind, endpoint, model, capability, policy, and enabled validation.
- [ ] Run the same repository contract on PostgreSQL/MySQL. Add three explicit CI steps immediately after RSS schema verification:

```yaml
- name: Verify AI provider storage on SQLite
  run: cargo test --locked --test ai_provider_storage sqlite -- --nocapture --test-threads=1
- name: Verify AI provider storage on PostgreSQL
  run: cargo test --locked --test ai_provider_storage postgres -- --nocapture --test-threads=1
- name: Verify AI provider storage on MySQL
  run: cargo test --locked --test ai_provider_storage mysql -- --nocapture --test-threads=1
```

- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_storage sqlite -- --nocapture --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Explicitly stage repository, storage test, module export, and CI workflow; commit and push `feat: load scoped ai provider bindings`.

### Task 5: DNS-pinned HTTPS transport

**Files:**

- Create: `src/content/provider/transport/mod.rs`
- Create: `src/content/provider/transport/dns.rs`
- Create: `src/content/provider/transport/http.rs`
- Create: `src/content/provider/transport/body.rs`
- Create: `src/content/provider/transport/types.rs`
- Modify: `src/content/provider/mod.rs`

**Interfaces:**

- Produces public `ProviderTransport` trait, `HttpsProviderTransport`, `ProviderTransportResponse`, `ProviderTransportError`, `ProviderTransportErrorKind`, `ProviderTimeoutStage`, and `ProviderRetryAfter`.
- Private injection traits `DnsResolver`, `HttpExecutor`, and `HttpBody` allow unit abuse tests without external network access.
- Consumes only validated `ProviderEndpoint` and adapter-produced `EncodedProviderRequest`; performs exactly one POST.

- [ ] In `types.rs`, write redacted error/response contracts first. `ProviderTransportError::Debug` contains provider ID, kind/stage/count only; reqwest sources use `without_url()`.
- [ ] In `dns.rs`, write fake-resolver tests for public IPv4/IPv6, standard NAT64, literal address, empty/17-answer/mixed-private/private/documentation/duplicate results, exact 3-second timeout, deduplication, and effective port. Implement hickory A+AAAA resolution and all-address fail-closed approval with `AddressPolicy::public_only()`.
- [ ] In `http.rs`, write fake-executor tests for forbidden hop-by-hop/host/content-length/proxy headers, invalid secret/public header values, no proxy/redirect/decompression builder settings, connect/first-byte timeout mapping, exact POST body, missing/mismatched peer, redirect body not polled, and one executor call.
- [ ] In `body.rs`, write fake-body and deterministic compressor tests for identity/gzip/Brotli/zlib success, multiple/unknown encodings, content-length early rejection, 2 MiB compressed/decoded boundaries, 100x expansion boundary, empty chunks yielding, idle/total deadline, and reqwest body error redaction.
- [ ] In `mod.rs`, orchestrate total deadline, endpoint join, DNS approval, header conversion, first-byte execution, peer check, single `Retry-After`, redirect denial, non-2xx body discard, and 2xx bounded decode. Use 90s total, 5s connect, 20s first byte, 10s idle, 2 MiB compressed/decoded, and 16 DNS answers exactly.
- [ ] Add unit tests proving invalid/multiple `Retry-After` is `ResponseHeaders`, delta/date values preserve a UTC deadline, and non-2xx response bodies are never polled.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked content::provider::transport -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [ ] Explicitly stage only transport files and module export; commit and push `feat: execute ai providers through pinned https`.

### Task 6: Provider client composition and stable call errors

**Files:**

- Create: `src/content/provider/client.rs`
- Modify: `src/content/provider/mod.rs`
- Create: `tests/ai_provider_transport.rs`

**Interfaces:**

- Produces `ProviderClient<T: ProviderTransport>`, `ProviderCallError`, and `ProviderCallErrorKind`.
- Consumes `ProviderBinding`, existing adapter encode/decode methods, and Task 5 transport.
- Returns only the existing `StructuredGenerationResponse`; provider wire data does not escape.

- [ ] Write a recording fake transport that captures only safe request facts in assertions and returns the eight existing provider fixture bodies/statuses. Never implement `Debug` for the fake captured secret headers.
- [ ] Add four success tests proving each binding uses its immutable kind/default or prefixed endpoint, exact adapter path/header/body, configured model, and canonical decoder result.
- [ ] Add rejection tests for request model mismatch, policy output-token overflow, invalid credential header bytes, disabled/corrupt binding construction barriers, transport timeout/network/redirect/peer/body errors, adapter authentication/rate/rejected/upstream/malformed/schema errors, and retry deadline propagation.
- [ ] Implement `ProviderClient::generate`: validate binding/request match, call `kind.encode_request(request, credential.clone())`, invoke transport once, then call `kind.decode_response(binding.model(), status, body)`. Map errors without retaining encoded request/response.
- [ ] Implement stable call kinds `InvalidRequest`, `RequestTooLarge`, `Transport`, `Timeout`, `Authentication`, `RateLimited`, `Rejected`, `Upstream`, `ResponseTooLarge`, `MalformedResponse`, and `OutputSchemaInvalid`. `Debug`/`Display` contain no endpoint, model, secret, request, response, prompt, schema, or output.
- [ ] Add a source-boundary test proving provider wire keys remain confined to adapter modules/tests/fixtures and transport does not branch on provider names beyond `ProviderKind` dispatch.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_transport -- --nocapture
cargo test --locked --test ai_provider_adapters -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [ ] Explicitly stage client, module export, and integration test; commit and push `feat: compose ai provider execution`.

### Task 7: Operator documentation, full gates, report, push, and CI

**Files:**

- Create: `docs/ai-providers.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/ai-provider-core-v1-report.md`
- Modify: `docs/superpowers/plans/2026-07-18-ai-provider-core-v1.md`

**Interfaces:**

- Produces exact operator key-generation/rotation rules and endpoint/transport guarantees.
- Produces delivery evidence and names the next slice as provider administration API/UI plus content jobs/artifacts, without claiming the AI plugin is complete.

- [ ] Document `RAINDROP_PROVIDER_SECRET_KEYS`, TOML equivalent, active-first rotation, retention of previous keys, 32-byte URL-safe no-padding generation examples, backup implications, startup-without-key behavior, and recovery by re-entering credentials. Examples contain generated placeholders only.
- [ ] Document default endpoints, custom HTTPS path prefixes, private IP/redirect/proxy rejection, DNS pinning, timeouts, response limits, non-streaming status, and the remaining live provider contract probe.
- [ ] Run a staged secret scan and source-boundary scan. Confirm no literal matching the test keys/credentials appears outside test files and no `encrypted_secret`/`ExposeSecret` use escapes provider config/secret/repository/transport/client boundaries.
- [ ] Run fresh local gates:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test ai_provider_secret -- --nocapture
cargo test --locked --test ai_provider_storage sqlite -- --nocapture --test-threads=1
cargo test --locked --test ai_provider_transport -- --nocapture
cargo test --locked --test ai_provider_adapters -- --nocapture
cargo test --locked --all-features
git diff --check
```

- [ ] If local PostgreSQL/MySQL URLs are configured, run the two provider storage contracts; otherwise rely on the explicit CI services and state that fact in the report.
- [ ] Record exact test counts, schema/index/FK facts, key/envelope/redaction evidence, transport abuse coverage, dependency/lockfile delta, commits, explicit remaining work, and existing advisories in `.superpowers/sdd/ai-provider-core-v1-report.md`.
- [ ] Update `tasks/plan.md` to point to provider administration API/UI plus content jobs/artifacts as the next AI slice. Keep all AI/plugin/MCP todo items unchecked because no user-visible content operation exists.
- [ ] Explicitly stage docs/report/task files; commit and push `test: verify ai provider core`.
- [ ] Monitor the triggered CI only for concrete failures. On success, append run ID and job matrix to the report, mark this checkbox complete, commit `docs: record ai provider core ci [skip ci]`, and push. Do not initiate additional review loops after a green full matrix.

## Plan self-review

- Spec coverage: dedicated key config, AEAD envelope, three-database record, immutable kind, endpoint contract, capability/policy validation, scope/revision repository, DNS/IP/peer/redirect/time/body transport, client composition, docs, gates, push, and CI each map to one task.
- Dependency order: keyring precedes encrypted storage; schema precedes domain conversion; model precedes repository; repository/adapter contracts precede transport client; full gates follow all implementation commits.
- Type consistency: `ProviderKind`, `ProviderScope`, `ProviderEndpoint`, `ProviderCapabilities`, `ProviderPolicy`, `ProviderMetadata`, `ProviderBinding`, `ProviderTransport`, and response/error types retain the same names/signatures across tasks.
- DDIA: the database is the record system; response streams are transient; update uses optimistic revision; policy facts are normalized; future fields are additive.
- Security: no session-secret derivation, plaintext storage, raw URL execution, redirect/proxy, private DNS answer, unpinned peer, unbounded body/decompression, raw body error, or secret-bearing formatted type remains.
- Scope: no provider API/UI, job, artifact, plugin, MCP, streaming, live credential, or completion claim is included.
- Placeholder scan: no TBD, unspecified validation, generic error-handling instruction, subagent dispatch, or unowned interface remains.
