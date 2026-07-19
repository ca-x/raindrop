# Official AI Plugin Contract / Registry Core v1 Design

Date: 2026-07-19

Status: binding implementation slice

Parent specification: `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`

## 1. Objective

Deliver the first executable boundary of Raindrop's plugin ecosystem: a versioned content-plugin WIT package, canonical official AI-plugin manifest/config/artifact/lifecycle assets, fail-closed Rust validators, and a tenant-safe registry record system on SQLite, PostgreSQL, and MySQL.

This slice makes plugin identity, compatibility, signatures, configuration, grants, and reserved KV state durable and testable before any Wasmtime host is allowed to execute code. It does not claim that summaries, translations, MCP calls, lifecycle dispatch, or the official Wasm component are runnable yet.

Success means a later runtime can only begin from a cryptographically verified bundled component descriptor and user-scoped registry records; it cannot invent a native summary/translation processor or bypass the content-job core.

## 2. Assumptions and delegated review

- The user delegated specification review and confirmation to the main Agent and requested no sub-agent development. This specification receives one bounded internal review and then is implemented inline.
- `raindrop.ai-content` remains the only v1 business plugin and the only allowed distribution is `BUNDLED_OFFICIAL`.
- The existing Provider Core and Content Jobs / Artifacts Core are authoritative. This slice neither duplicates their tables nor calls a provider.
- The current worktree and branch remain `feature/foundation-bootstrap`; `.superpowers/research/` and the root `node_modules/` remain untouched.
- Rust stays on edition 2024 with `rust-version = 1.94`.

## 3. Scope

### 3.1 Included

1. WIT package `raindrop:content-plugin@1.0.0` with world `content-plugin-v1`, exports `descriptor`, `execute`, and `on-event`, and imports `host-ai.generate-structured` and `host-mcp.call-tool`.
2. Canonical contract assets:
   - official manifest template;
   - AI content configuration JSON Schema;
   - summary and translation artifact JSON Schemas;
   - one fixture for each of the five feed lifecycle events.
3. Structural and semantic validators for production manifests, AI config, artifact payloads, lifecycle fixtures, identifiers, hashes, canonical JSON, and size limits.
4. SHA-256 component digest verification and Ed25519 release-signature verification against an injected keyring.
5. Four durable tables and SeaORM entities:
   - `plugin_installations`;
   - `plugin_configs`;
   - `plugin_capability_grants`;
   - `plugin_kv`.
6. Repository operations for bundled installation synchronization, user configuration replacement/read, capability grant/revoke/list, and quota-bounded KV get/put/delete.
7. SQLite/PostgreSQL/MySQL contract tests and a WIT parser gate.

### 3.2 Excluded

- Wasmtime, component instantiation, fuel/memory/epoch enforcement, WASI configuration, or guest bindings.
- A compiled or embedded `raindrop.ai-content.wasm` component.
- Provider broker composition, prompt implementation, summary/translation execution, artifact Reader sidecar, or content APIs.
- Lifecycle dispatcher/delivery tables or execution of synchronous hooks.
- MCP client transport, MCP server, tool discovery, tool calls, or MCP credentials.
- Plugin install/upload/download APIs, marketplace, arbitrary sideloading, SDK publication, or management UI.
- Production release signing keys. Tests use deterministic test-only keys; future release packaging injects real public keys and signed assets.

## 4. Trust boundaries and abuse cases

| Boundary | Abuse | Required control |
| --- | --- | --- |
| manifest JSON -> registry | duplicate keys, oversized JSON, wrong plugin identity, placeholder manifest persisted | bounded unique-key JSON parser; exact official identity; reject `valueSource`; require digest/signature `value` |
| component bytes -> installation | manifest claims a different component | SHA-256 of supplied bytes must equal lower-hex `componentDigest.value` |
| signature -> trusted descriptor | attacker substitutes manifest/component or unknown key | exact key ID lookup; Ed25519 verification over the v1 framed canonical payload |
| config JSON -> content jobs | secrets, unknown fields, invalid cross-field combinations, unstable hash | strict typed validation; canonical JSON; bounded arrays/text; BLAKE3 config hash; no credential/header/endpoint fields |
| repository -> multi-user state | cross-tenant read/update or resource enumeration | every config/grant/KV query binds plugin ID and owner user ID; missing and invisible records share `NotFound` |
| concurrent writers -> config/grant/KV | lost update or quota bypass | optimistic revision for config/grant; user-first transaction for KV quota checks; unique constraints as final arbiter |
| stored JSON -> runtime | corrupt rows create ambient authority | parse and revalidate on every read; corrupt data fails closed with redacted errors |
| lifecycle fixtures -> future guest | raw feed body, secrets, unstable fields | fixtures contain stable IDs/counts/safe metadata only; no body, credential, header value, query string, or user secret |

Model output, feed text, MCP metadata, and plugin output remain untrusted in later slices. A valid signature proves release origin, not that guest output is safe.

## 5. Contract asset layout

```text
contracts/
  wit/raindrop-content-plugin-v1/
    types.wit
    host-ai.wit
    host-mcp.wit
    content-plugin.wit
    world.wit
  plugins/raindrop.ai-content/
    manifest.template.json
    config.v1.schema.json
  artifacts/
    ai-summary.v1.schema.json
    ai-translation.v1.schema.json
  lifecycle/feed-refresh-v1/
    feed.refresh.before.json
    feed.refresh.fetched.json
    entry.process.json
    feed.refresh.persisted.json
    feed.refresh.completed.json
```

`wit-parser 0.253.0` is a dev dependency with default features disabled and `std` enabled. CI parses the directory and selects `content-plugin-v1`; string matching alone is not a sufficient WIT gate.

## 6. Production manifest contract

The template remains human/release tooling input and contains `valueSource = RELEASE_BUILD` for digest and signature. It is never a production installation.

A production manifest:

- is UTF-8 JSON no larger than 64 KiB;
- has no duplicate object keys at any depth;
- has `manifestVersion = 1`;
- has exact `pluginKey = raindrop.ai-content`, `distribution = BUNDLED_OFFICIAL`, `version = 1.0.0`, and `abi = raindrop:content-plugin@1.0.0` in this slice;
- exposes operations `summarize`, `translate` exactly once each;
- subscribes only to `feed.refresh.persisted` schema version 1;
- requires only `ai.generate_structured`, optionally requests only `mcp.call_tool`, and has an empty `ambientPermissions` array;
- references the committed config and artifact schema IDs;
- contains lower-hex SHA-256 `componentDigest.value` and no digest `valueSource`;
- contains base64url-no-pad Ed25519 `signature.value`, a known `keyId`, and no signature `valueSource`.

Unknown additive fields remain part of signature verification and canonical persistence but are ignored by the typed v1 view.

### 6.1 Signature payload

The exact signed byte sequence is:

```text
UTF8("raindrop.plugin-signature.v1")
u64be(len(canonicalManifestWithoutSignatureValue))
canonicalManifestWithoutSignatureValue
u64be(len(componentDigestLowerHex))
UTF8(componentDigestLowerHex)
```

`canonicalManifestWithoutSignatureValue` recursively sorts object keys, preserves array order and scalar values, removes only `signature.value`, and serializes compact JSON. The digest remains inside the canonical manifest and is framed again to make the security contract explicit and independently testable.

## 7. WIT compatibility boundary

The v1 WIT package uses kebab-case field/function names and contains no resource that exposes a socket, file, environment, process, database, provider secret, provider URL, MCP credential, or raw transport.

- `content-plugin.descriptor` returns identity, supported operations, lifecycle subscriptions, and requested capabilities.
- `content-plugin.execute` consumes a host-built operation request and returns an artifact candidate or stable plugin error.
- `content-plugin.on-event` consumes a `lifecycle-request` containing verified plugin identity, canonical per-user config snapshot/hash, and one versioned lifecycle event, then returns declarative intents only.
- `host-ai.generate-structured` accepts a host-issued provider binding ID, bounded untrusted JSON, output schema, and token ceilings.
- Operation requests carry complete non-secret host-issued tool descriptors: binding ID, configured connection/tool identity, bounded untrusted description, canonical input schema, and schema digest. `host-mcp.call-tool` still accepts only the binding ID, canonical JSON arguments, and a bounded timeout; the descriptor grants no authority.

The corrected invocation record shapes are bound by `docs/superpowers/specs/2026-07-19-official-ai-component-invocation-contract-v1-design.md`. They replace the earlier pre-release ID-only tool binding and event-only `on-event` shape before any executable plugin release consumed the ABI.

The WIT contract does not grant capabilities. The future host rechecks current invocation grants for every import call.

## 8. JSON contracts

### 8.1 AI config

The committed schema ID remains `raindrop://schemas/plugins/raindrop.ai-content/config/v1`. Rust validation implements the same field set and cross-field rules from the parent specification:

- unknown fields fail;
- `schemaVersion` is exactly 1;
- both operation objects exist;
- provider IDs are canonical UUIDs;
- BCP 47 locale normalization is deterministic;
- `DISABLED` MCP requires zero calls and no tools;
- `CONTEXT_ENRICHMENT` requires 1..=4 calls and 1..=16 unique exact `(connectionId, toolName)` bindings;
- automatic mode requires an enabled operation and an explicit all/feed/category scope;
- automatic translation requires translate enabled;
- canonical config JSON is at most 256 KiB.

Configuration contains no secret, endpoint, header, raw prompt, or provider credential. Provider/feed/category/connection ownership resolution belongs to a later application service because those resources do not all exist in this slice.

### 8.2 Artifacts

- Summary schema ID: `raindrop://schemas/artifacts/ai-summary/v1`.
- Translation schema ID: `raindrop://schemas/artifacts/ai-translation/v1`.
- Both payloads reject unknown fields and raw HTML in Markdown fields.
- Summary arrays are bounded and contain non-empty control-free text.
- Translation requires normalized source/target locales and bounded title/body Markdown.
- Canonical artifact JSON is at most 512 KiB.

These validators produce candidates only; persistence still goes through the existing `ContentJobRepository::complete_success` path in a later worker slice.

### 8.3 Lifecycle fixtures

All fixtures use `schemaVersion = 1` and an envelope containing `eventId`, `eventType`, `refreshId`, `sequence`, `occurredAt`, and `idempotencyKey`. Event-specific context is bounded and contains no raw response body or unclean HTML. The persisted/completed sequences remain 10/20; the other fixtures use 1/5/8 to freeze relative lifecycle ordering without changing the existing outbox contract.

## 9. Durable data model

### 9.1 `plugin_installations`

| Column | Contract |
| --- | --- |
| `id` | UUID string primary key |
| `plugin_key` | canonical lowercase plugin key; unique |
| `version` | exact manifest version |
| `abi_version` | exact WIT package identifier |
| `distribution` | `BUNDLED_OFFICIAL` only in v1 |
| `component_digest` | lower-hex SHA-256 |
| `manifest_json` | canonical production manifest |
| `signature_key_id` / `signature` | verified Ed25519 metadata |
| `system_state` | `ENABLED`, `DISABLED`, or `QUARANTINED` |
| `revision` | optimistic system-state/version revision |
| `installed_at` / `updated_at` | operational UTC timestamps |

Bundled sync is idempotent. Exact manifest/component replay returns the existing row. A version/digest update increments revision in one transaction. A `plugin_key` collision with a different distribution fails closed. This slice has no delete/uninstall operation.

### 9.2 `plugin_configs`

One row per `(plugin_id, owner_user_id)`. It stores canonical config JSON/hash, schema version, enable state, optimistic revision, and timestamps. User deletion cascades; installation deletion is restricted because uninstall is outside v1.

Replacing config requires the expected revision. Creation uses expected revision `None`; update uses `Some(current)`. Concurrent stale writers receive `RevisionConflict` rather than silently losing settings.

### 9.3 `plugin_capability_grants`

One authoritative row per hashed canonical grant key:

```text
pluginId, ownerUserId, capability, operation, resourceType, resourceId
```

The table stores the raw bounded fields, a BLAKE3 `grant_key_hash`, canonical `constraints_json`, optimistic revision, `created_at`, `updated_at`, and nullable `revoked_at`. Regranting updates the existing row and clears `revoked_at`; it does not create duplicate active authority. Grants never store provider/MCP secrets.

### 9.4 `plugin_kv`

The table key is `(plugin_id, owner_user_id, key)`. Keys use lowercase ASCII `[a-z0-9][a-z0-9._/-]{0,127}`. Values are opaque bytes with a 64 KiB per-value limit. Each plugin/user scope permits at most 128 keys and 1 MiB total bytes. Writes run in a short transaction, lock/validate the owner and plugin scope first, compute the post-write quota, then upsert. A revision and timestamps support future runtime observation.

`plugin_kv` is reserved infrastructure. The official AI manifest does not request a KV capability, and this slice does not call it from any runtime.

## 10. Repository interface

The root module is `raindrop::plugins` and exports stable domain types plus `PluginRegistryRepository`.

```rust
pub struct OfficialSigningKey {
    pub key_id: String,
    pub public_key: [u8; 32],
}

pub struct BundledOfficialPlugin;

impl BundledOfficialPlugin {
    pub fn verify(
        manifest_json: &[u8],
        component: &[u8],
        keys: &[OfficialSigningKey],
    ) -> Result<Self, PluginRegistryError>;
}

impl PluginRegistryRepository {
    pub async fn sync_bundled(
        &self,
        bundle: &BundledOfficialPlugin,
    ) -> Result<PluginInstallation, PluginRegistryError>;

    pub async fn replace_ai_config(
        &self,
        plugin_key: &str,
        user_id: &str,
        expected_revision: Option<u64>,
        is_enabled: bool,
        config_json: &[u8],
    ) -> Result<PluginConfig, PluginRegistryError>;

    pub async fn get_ai_config(
        &self,
        plugin_key: &str,
        user_id: &str,
    ) -> Result<Option<PluginConfig>, PluginRegistryError>;

    pub async fn grant_capability(...)
        -> Result<PluginCapabilityGrant, PluginRegistryError>;
    pub async fn revoke_capability(...)
        -> Result<PluginCapabilityGrant, PluginRegistryError>;
    pub async fn list_active_grants(...)
        -> Result<Vec<PluginCapabilityGrant>, PluginRegistryError>;
    pub async fn get_kv(...)
        -> Result<Option<PluginKvValue>, PluginRegistryError>;
    pub async fn put_kv(...)
        -> Result<PluginKvValue, PluginRegistryError>;
    pub async fn delete_kv(...)
        -> Result<(), PluginRegistryError>;
}
```

The concrete grant/KV argument structs carry plugin key, user ID, operation/resource identity, expected revision where applicable, and owned bounded values. Public errors expose only stable kinds; `Debug`, `Display`, and error chains do not echo manifest, config, constraints, KV bytes, signature bytes, component bytes, or tenant IDs.

## 11. Consistency and evolution

- The relational database is the record system for installations, configs, grants, and KV. Parsed manifests, component bytes, Wasmtime stores, provider responses, and MCP transports are transient.
- Unique constraints arbitrate identity races; optimistic revisions prevent lost updates; repository reads revalidate stored data.
- All SQL remains portable across SQLite/PostgreSQL/MySQL. No partial indexes, JSON-native columns, backend-specific generated columns, or database collation assumptions are used.
- The plugin key grammar is lowercase ASCII, avoiding MySQL case-folding ambiguity.
- Schema evolution is additive. Breaking WIT changes require a new major package; persisted JSON always retains explicit schema versions.
- Feed transactions never touch these tables except a future post-commit dispatcher outside the original refresh transaction.

## 12. Commands and project structure

```text
Build:  cargo check --all-targets
Format: cargo fmt --all -- --check
Lint:   cargo clippy --all-targets --all-features -- -D warnings
Test:   cargo test --all-targets
```

New Rust source lives in focused files under `src/plugins/`; migration and entities remain under the existing `src/db` layout. Integration tests use public interfaces and the shared three-backend database harness.

## 13. Test seams

The approved public seams are fixed by this specification, so TDD does not require another user confirmation:

1. `BundledOfficialPlugin::verify` for JSON uniqueness, identity, digest, signature, and redaction behavior.
2. `AiContentConfig::parse`, summary/translation artifact parsers, and lifecycle fixture parser for semantic contracts.
3. `wit_parser::Resolve::push_dir` plus world selection for WIT syntax/resolution.
4. `PluginRegistryRepository` for installation sync, user isolation, revisions, grant lifecycle, KV quotas, and corruption fail-closed behavior.
5. SeaORM migration/entities for schema, constraints, round trips, rollback, and all three database backends.

## 14. Boundaries

- Always: validate before persistence; canonicalize before hashing/signing; bind every user record query to owner and plugin; use parameterized SeaORM queries; run targeted tests before each commit; push immediately after each commit.
- Internally review once: migration/index design, signature frame, public types/errors, config/grant/KV limits, and new dependency.
- Never: persist a manifest template; accept an unknown signing key; log untrusted JSON/bytes/signatures; add a native summary/translation processor; execute Wasm/provider/MCP; edit `.superpowers/research/` or root `node_modules/`; use sub-agents; use `git add -A`.

## 15. Completion criteria

1. The WIT directory parses and resolves to package `raindrop:content-plugin@1.0.0` and world `content-plugin-v1` in CI.
2. A correctly signed production manifest and matching component produce a validated bundle; placeholder, duplicate-key, wrong-digest, unknown-key, and invalid-signature inputs fail closed without echo.
3. Config, artifact, and five lifecycle fixtures pass exact semantic validation; hostile/oversized/unknown-field cases fail.
4. All four tables migrate, index, round-trip, roll back, and reapply on SQLite; repository contracts pass on SQLite/PostgreSQL/MySQL.
5. Config/grant revisions reject stale writes; cross-user reads are hidden; KV enforces 64 KiB/128-key/1 MiB limits atomically.
6. No route, worker, runtime, provider call, MCP transport, or Reader claim is added in this slice.
7. `tasks/plan.md` and `tasks/todo.md` distinguish completed Provider Core, Content Job Core, Plugin Contract/Registry Core, and still-pending Wasmtime/AI/MCP/UI work.
8. Formatting, clippy, targeted tests, and `cargo test --all-targets` pass before the final commit is pushed.

## 16. Bounded internal review conclusion

- DDIA: normalized relational records, explicit identity constraints, optimistic concurrency, portable indexes, and short transactions keep the database authoritative without introducing a second scheduler or mutable derived flag.
- API design: versioned additive contracts, one canonical plugin identity, typed boundary validation, stable redacted errors, and an explicit signature frame make the interface difficult to misuse.
- Security: no ambient authority, no secrets in config/grants/fixtures, duplicate-key rejection, exact digest/signature verification, tenant binding, bounded data, and corruption revalidation address the first executable plugin trust boundary.
- Scope: the slice is independently valuable and testable but makes no false claim that Wasmtime, the official component, summary/translation, lifecycle execution, or MCP is complete.
