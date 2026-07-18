# Official AI Plugin Contract / Registry Core v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `executing-plans` to implement this plan task-by-task. This plan intentionally forbids sub-agent execution per user instruction. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a cryptographically verified, versioned plugin contract and tenant-safe durable registry that future Wasmtime/AI/MCP slices must use.

**Architecture:** Canonical files under `contracts/` define the WIT and JSON contracts. Focused Rust modules under `src/plugins/` parse and validate those contracts, verify official bundles, and expose a registry repository backed by four portable SeaORM tables. No runtime execution is introduced.

**Tech Stack:** Rust 2024 / Rust 1.94, Serde/Serde JSON, ring SHA-256 + Ed25519, BLAKE3, SeaORM/SeaORM Migration, `wit-parser = 0.253.0` as a test-only parser.

## Global Constraints

- Keep `raindrop.ai-content` as the only v1 business plugin and `BUNDLED_OFFICIAL` as the only v1 distribution.
- Provider and content-job cores remain the only model-call/job/artifact boundaries.
- Use public seam tests and red-green cycles; do not test private functions directly.
- Use SQLite/PostgreSQL/MySQL portable schema and repository semantics.
- Do not touch `.superpowers/research/`, root `node_modules/`, routes, Reader UI, Wasmtime, provider execution, MCP transport, or lifecycle dispatch.
- Do not use sub-agents or `git add -A`; push immediately after every commit.

---

### Task 1: Commit the binding specification and execution plan

**Files:**
- Create: `docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md`
- Create: `docs/superpowers/plans/2026-07-19-official-ai-plugin-contract-registry-core-v1.md`

**Interfaces:**
- Consumes: parent plugin design and completed provider/content-job contracts.
- Produces: exact scope, schemas, public seams, security frame, persistence semantics, and verification gates for all later tasks.

- [ ] Verify the documents contain no unresolved placeholder with:
  `rg -n 'T[B]D|T[O]DO|implement lat[e]r|fill in det[a]ils|Similar to T[a]sk' docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md docs/superpowers/plans/2026-07-19-official-ai-plugin-contract-registry-core-v1.md`
- [ ] Review the parent specification headings 6-12 and confirm every in-scope contract maps to Tasks 2-5.
- [ ] Commit only the two documents with `git add <two exact paths> && git commit -m "docs: design official plugin registry core"`.
- [ ] Push `feature/foundation-bootstrap` immediately.

### Task 2: Add parseable WIT and canonical JSON contract assets

**Files:**
- Create: `contracts/wit/raindrop-content-plugin-v1/types.wit`
- Create: `contracts/wit/raindrop-content-plugin-v1/host-ai.wit`
- Create: `contracts/wit/raindrop-content-plugin-v1/host-mcp.wit`
- Create: `contracts/wit/raindrop-content-plugin-v1/content-plugin.wit`
- Create: `contracts/wit/raindrop-content-plugin-v1/world.wit`
- Create: `contracts/plugins/raindrop.ai-content/manifest.template.json`
- Create: `contracts/plugins/raindrop.ai-content/config.v1.schema.json`
- Create: `contracts/artifacts/ai-summary.v1.schema.json`
- Create: `contracts/artifacts/ai-translation.v1.schema.json`
- Create: five files under `contracts/lifecycle/feed-refresh-v1/`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `tests/plugin_contract_assets.rs`

**Interfaces:**
- Consumes: package/world/function/schema/event names from the binding specification.
- Produces: files consumed by `src/plugins/contracts.rs` and future Wasmtime component bindings.

- [ ] Write `tests/plugin_contract_assets.rs` to call `wit_parser::Resolve::push_dir`, select `content-plugin-v1`, parse every JSON asset, assert exact schema IDs, and assert the manifest template still uses `valueSource` rather than a fake production value.
- [ ] Run `cargo test --test plugin_contract_assets`; expected red result is the missing assets/dev dependency.
- [ ] Run `cargo add wit-parser@0.253.0 --dev --no-default-features --features std` and add the exact WIT/JSON files.
- [ ] Run `cargo test --test plugin_contract_assets`; expected result is all asset contract tests passing.
- [ ] Commit the exact asset/test/dependency paths with message `feat: define content plugin contract assets` and push.

### Task 3: Add fail-closed contract validators and bundle verification

**Files:**
- Create: `src/plugins/mod.rs`
- Create: `src/plugins/error.rs`
- Create: `src/plugins/json.rs`
- Create: `src/plugins/manifest.rs`
- Create: `src/plugins/config.rs`
- Create: `src/plugins/artifact.rs`
- Create: `src/plugins/lifecycle.rs`
- Modify: `src/lib.rs`
- Create: `tests/plugin_contract_validation.rs`

**Interfaces:**
- Consumes: committed contract assets and injected `OfficialSigningKey` values.
- Produces: `BundledOfficialPlugin::verify`, `AiContentConfig::parse`, `SummaryArtifact::parse`, `TranslationArtifact::parse`, `LifecycleEvent::parse`, stable getters, canonical JSON/hash values, and redacted `PluginRegistryErrorKind`.

- [ ] Write one manifest red test covering valid deterministic Ed25519 verification plus duplicate keys, placeholder fields, wrong digest, unknown key, bad signature, invalid identity, and error redaction.
- [ ] Implement unique-key bounded JSON parsing, recursive canonicalization, SHA-256 digest comparison, exact signature framing, key lookup, and Ed25519 verification until the manifest test is green.
- [ ] Write config red tests for exact valid config, unknown fields, invalid UUID/locale/MCP cross-field rules, duplicate tool bindings, automatic-scope rules, secret-like unsupported fields, and size limits.
- [ ] Implement strict Serde DTOs plus semantic validation and BLAKE3 config hashing until config tests are green.
- [ ] Write artifact/lifecycle red tests for exact fixtures, unknown fields, raw HTML/script protocols, wrong sequences/types, oversized values, and redacted errors.
- [ ] Implement artifact/lifecycle parsers and stable getters until all tests pass.
- [ ] Run `cargo test --test plugin_contract_validation --test plugin_contract_assets`.
- [ ] Commit exact validator/module/test paths with message `feat: verify official plugin contracts` and push.

### Task 4: Add the portable plugin registry schema and entities

**Files:**
- Create: `src/db/migration/plugins.rs`
- Modify: `src/db/migration.rs`
- Create: `src/db/entities/plugin_installation.rs`
- Create: `src/db/entities/plugin_config.rs`
- Create: `src/db/entities/plugin_capability_grant.rs`
- Create: `src/db/entities/plugin_kv.rs`
- Modify: `src/db/entities.rs`
- Create: `tests/plugin_registry_migration.rs`

**Interfaces:**
- Consumes: validated storage field bounds from Task 3.
- Produces: four tables/entities and named portable indexes for Task 5.

- [ ] Write the SQLite migration red test to inspect every table/column/index, round-trip representative records, verify unique/cascade/restrict behavior, roll back all four tables, and reapply migrations.
- [ ] Implement `CreatePluginRegistry` after `CreateContentJobs`, creating installation -> config/grant/KV tables in dependency order and dropping them in reverse order.
- [ ] Use named unique/index contracts for plugin key, config owner, grant key hash, active grant lookup, and KV owner scope; use operational microsecond timestamps on all three backends.
- [ ] Add focused SeaORM entities and module exports.
- [ ] Run `cargo test --test plugin_registry_migration --test database_migrations`.
- [ ] Commit exact migration/entity/test paths with message `feat: persist plugin registry records` and push.

### Task 5: Implement tenant-safe registry repository contracts

**Files:**
- Create: `src/plugins/model.rs`
- Create: `src/plugins/repository.rs`
- Modify: `src/plugins/mod.rs`
- Create: `tests/plugin_registry_repository.rs`
- Create: `tests/plugin_registry_backend_contracts.rs`

**Interfaces:**
- Consumes: `BundledOfficialPlugin`, config/grant/KV validators, four SeaORM entities.
- Produces: `PluginRegistryRepository::{sync_bundled,get_installation,replace_ai_config,get_ai_config,grant_capability,revoke_capability,list_active_grants,get_kv,put_kv,delete_kv}` and stable domain models.

- [ ] Write the SQLite tracer test for signed bundle sync, exact replay, version/digest update revision, template rejection before storage, and corrupt-row fail-closed behavior.
- [ ] Implement installation synchronization with a transaction and unique constraint arbitration.
- [ ] Write config tests for create/read/update, stale revision conflict, disabled/missing user, cross-user isolation, and stored JSON corruption.
- [ ] Implement canonical config persistence and owner-bound reads with optimistic revision.
- [ ] Write grant tests for deterministic key identity, exact active list, regrant/revoke revisions, stale conflicts, no secret echo, and tenant isolation.
- [ ] Implement grant hash/upsert/revoke/list behavior.
- [ ] Write KV tests for get/put/delete, 64 KiB value, 128-key limit, 1 MiB total limit, replacement accounting, stale owner/plugin rejection, concurrent quota convergence, and tenant isolation.
- [ ] Implement short owner-first KV transactions and quota checks.
- [ ] Run SQLite tests, then expose the same repository contract through `tests/plugin_registry_backend_contracts.rs` for SQLite/PostgreSQL/MySQL.
- [ ] Run `cargo test --test plugin_registry_repository --test plugin_registry_backend_contracts`.
- [ ] Commit exact repository/model/test paths with message `feat: manage plugin registry state` and push.

### Task 6: Record completion boundaries and run final gates

**Files:**
- Modify: `tasks/plan.md`
- Modify: `tasks/todo.md`
- Create: `.superpowers/sdd/official-ai-plugin-contract-registry-core-v1-report.md`
- Modify only if implementation discoveries changed a binding decision: the spec and this plan.

**Interfaces:**
- Consumes: verified Tasks 2-5.
- Produces: accurate project status that marks only contract/registry core complete and leaves runtime/component/AI/MCP/UI work pending.

- [ ] Update the global plan pointer and split AI/plugin checklist items into Provider Core, Content Job Core, Contract/Registry Core, Wasmtime/component, lifecycle, MCP, and UI sub-items.
- [ ] Write the SDD report with exact commits, files, limits, test counts, exclusions, and next dependency: Wasmtime Component Host / Broker Composition v1.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run the targeted plugin test set.
- [ ] Run `cargo test --all-targets`; the IT之家 live smoke may remain the single documented opt-in ignored test.
- [ ] Inspect `git status --short`, `git diff --check`, and the staged secret scan before committing.
- [ ] Commit exact docs/task/report paths with message `docs: record plugin registry core` and push.
