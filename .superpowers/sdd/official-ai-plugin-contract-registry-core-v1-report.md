# Official AI Plugin Contract / Registry Core v1 SDD Report

Date: 2026-07-19

Branch: `feature/foundation-bootstrap`

Binding specification: `docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md`

Execution plan: `docs/superpowers/plans/2026-07-19-official-ai-plugin-contract-registry-core-v1.md`

## Outcome

This slice establishes the first executable trust and persistence boundary for Raindrop plugins. Future AI content execution must start from a verified bundled descriptor and the durable registry; it may not add a native summary/translation processor or call a provider outside the existing content-job and ProviderClient boundaries.

The slice is complete as a contract/registry core. It deliberately does not claim that a Wasm component can execute, that summaries/translations are user-visible, that lifecycle hooks dispatch, or that MCP transport exists.

## Commits

- `873f85e docs: design official plugin registry core`
- `e67c05c docs: normalize plugin registry spec`
- `3665141 feat: define content plugin contract assets`
- `6d3561c feat: verify official plugin contracts`
- `404ea3e feat: persist plugin registry records`
- `495d202 feat: manage plugin registry state`

## Delivered contracts

### WIT

- Package: `raindrop:content-plugin@1.0.0`.
- World: `content-plugin-v1`.
- Guest exports: `descriptor`, `execute`, `on-event`.
- Host imports: `host-ai.generate-structured`, `host-mcp.call-tool`.
- No socket, file, environment, process, database, provider credential, MCP credential, or raw transport type is exposed.
- `wit-parser 0.253.0` resolves the committed package in tests.

### Canonical assets

- `contracts/plugins/raindrop.ai-content/manifest.template.json` remains a release template and cannot be persisted as an installation.
- AI config schema ID: `raindrop://schemas/plugins/raindrop.ai-content/config/v1`.
- Summary schema ID: `raindrop://schemas/artifacts/ai-summary/v1`.
- Translation schema ID: `raindrop://schemas/artifacts/ai-translation/v1`.
- Five lifecycle fixtures freeze before/fetched/entry.process/persisted/completed envelopes and relative sequences 1/5/8/10/20.

### Manifest trust boundary

- JSON is capped at 64 KiB and duplicate keys fail at every object depth.
- Production manifests require real digest/signature values and reject `valueSource` placeholders.
- Component bytes are SHA-256 checked against lower-hex manifest identity.
- Ed25519 verification uses an injected exact key-ID keyring.
- Signature input uses the frozen `raindrop.plugin-signature.v1` framed canonical payload.
- Unknown additive manifest fields remain signed and canonically persisted while the typed v1 reader ignores their semantics.
- Errors and debug output do not echo manifest bodies, component bytes, signature bytes, or untrusted identifiers.

### Config/artifact/lifecycle validators

- AI config rejects unknown fields, noncanonical UUIDs/locales, invalid token limits, inconsistent MCP mode/tool settings, duplicate exact tools, and invalid automatic scopes.
- Config JSON is canonicalized to at most 256 KiB and receives a domain-separated BLAKE3 hash used by future job identity.
- Summary/translation artifacts reject unknown fields, raw HTML delimiters, unsafe URL schemes, invalid locales, invalid counts, and payloads above 512 KiB.
- Lifecycle parsers validate event-specific strict contexts and never admit raw feed bodies or arbitrary extra context fields.

## Durable registry

### Tables

- `plugin_installations`
- `plugin_configs`
- `plugin_capability_grants`
- `plugin_kv`

All tables use portable SeaORM migrations and focused entities. Installation deletion is restricted; user deletion cascades only that user's config/grant/KV state. Migration rollback drops the tables in dependency order and supports reapply.

### Repository behavior

- `sync_bundled` is idempotent, preserves installation identity, and uses optimistic revision when a verified bundled version/digest changes.
- `replace_ai_config` uses user-first locking and create/update CAS semantics.
- Capability grants have deterministic framed identity, strict capability/operation/resource pairing, canonical secret-free constraints, in-place revoke/regrant, and optimistic revision.
- KV keys use `[a-z0-9][a-z0-9._/-]{0,127}`; values are capped at 64 KiB; each plugin/user scope is capped at 128 keys and 1 MiB total.
- KV writes lock the user first and calculate the post-write quota inside the transaction. At the exact concurrent key limit, one writer succeeds and one receives `QuotaExceeded`.
- Every user-scoped query binds both installation and owner. Missing/disabled users and invisible records fail without enumeration.
- Stored manifest/config/grant/KV data is revalidated on read and corrupt data fails closed.

## Verification evidence

Final local command:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Result:

- Formatting: pass.
- Clippy: pass with warnings denied.
- Tests discovered: 561.
- Tests passed: 560.
- Tests failed: 0.
- Tests ignored: 1.
- The ignored test is only `ithome_feed_securely_ingests_and_deduplicates`, which requires `RAINDROP_LIVE_RSS_SMOKE=1` and public network.
- SQLite plugin asset/validator/migration/repository/concurrency contracts ran locally.
- PostgreSQL/MySQL plugin repository tests are committed and wired to `RAINDROP_TEST_POSTGRES_URL` / `RAINDROP_TEST_MYSQL_URL`; those URLs were not configured in the local shell, so the release CI service jobs remain the final backend gate.
- The existing SeaORM-chain `proc-macro-error2 2.0.1` future-incompatibility warning remains tracked in `tasks/todo.md`; it is not introduced by this slice.

## Explicitly still pending

- Wasmtime Component Model host and guest binding generation.
- Embedded, release-signed `raindrop.ai-content.wasm` production component.
- Runtime fuel, memory, epoch, output, and ambient WASI enforcement.
- ProviderClient capability broker composition and prompt execution.
- Summary/translation worker and artifact sidecar/API/UI.
- Feed lifecycle dispatcher and delivery recovery.
- MCP client, MCP capability calls, and Raindrop MCP server.
- Plugin management API/UI, marketplace, third-party install, and SDK publication.

## Next dependency

The next implementation slice is `Wasmtime Component Host / Broker Composition v1`:

1. generate host bindings from the committed WIT;
2. instantiate only a verified bundled descriptor with no ambient WASI;
3. enforce fuel, 64 MiB linear memory, epoch deadline, request/output bounds, and stable trap mapping;
4. broker `ai.generate-structured` through `ProviderClient` and `mcp.call-tool` through a still-future MCP client interface;
5. execute only from a fenced `ContentJobClaim` and commit results only through `ContentRepository::complete_success` / `complete_failure`.
