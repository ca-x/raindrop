# Official AI Component v1 Design

Date: 2026-07-19

Status: internally approved for inline implementation

Parent specifications:

- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-wasmtime-component-host-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-ai-provider-broker-composition-v1-design.md`
- `docs/superpowers/specs/2026-07-19-official-ai-component-invocation-contract-v1-design.md`

## 1. Objective

Implement the real Rust guest component for `raindrop.ai-content@1.0.0`. The component must export the committed WIT world, produce summary and translation artifact candidates through `host-ai.generate-structured`, optionally perform bounded read-only MCP context enrichment through `host-mcp.call-tool`, and convert `feed.refresh.persisted` config/event snapshots into deterministic declarative content-job intents.

This slice proves real component bytes compile and execute inside the existing hardened Wasmtime host. It does not yet claim a production-signed embedded bundle, content worker, artifact commit, lifecycle dispatcher, MCP transport, HTTP API, or Reader sidecar.

## 2. Assumptions

1. The user delegated internal review and explicitly prohibited sub-agent development; implementation stays in the main Agent with one bounded review.
2. Rust remains edition 2024 with project MSRV 1.94.0.
3. `raindrop:content-plugin@1.0.0` is authoritative and is consumed through generated bindings only.
4. The component target is `wasm32-unknown-unknown`; it receives no WASI adapter, filesystem, environment, clock, random, socket, process, or inherited stdio.
5. `wit-bindgen 0.59.0` is pinned exactly. Its declared MSRV is 1.85.0, so it is below the project MSRV.
6. `cargo-component 0.21.1` is not used because its normal component pipeline includes a Preview 1 adapter path that is unnecessary for this no-WASI guest. The already pinned host-side `wit-component 0.253.0` componentizes the generated core Wasm in tests.
7. The lifecycle dispatcher will preselect users whose automatic rules may apply. The component still checks the config snapshot and returns intents only; the dispatcher/orchestrator rechecks feed/category scope, entry visibility, provider binding, quotas, and idempotency before enqueue.

## 3. Scope

### Included

1. A focused nested Rust workspace member under `plugins/official/raindrop-ai-content/`.
2. Generated guest imports/exports from the committed WIT directory.
3. Strict guest config/event/output parsing with canonical JSON construction.
4. Fixed descriptor matching the signed manifest contract.
5. Summary and translation prompt/input builders with exact committed artifact schemas.
6. A schema-aware MCP tool-plan request, bounded sequential calls, fail-open/fail-closed handling, and untrusted result projection into final generation.
7. Provider broker support for the bounded tool-plan schema family and typed tool-plan output validation.
8. Runtime validation that tool-plan schemas contain exactly the current host-issued binding IDs and exact input schemas.
9. Safe preservation of allowlisted guest failure codes across `PluginRuntimeError` without echoing arbitrary guest message text.
10. Deterministic componentization, no-WASI/source-confinement checks, and real Wasmtime execution tests.
11. CI installation of `wasm32-unknown-unknown` for the Rust job.

### Excluded

- Production release signing key material or a production manifest.
- Embedding/discovery of the component in the release binary.
- Content job worker composition, attempt terminal commit, or artifact persistence.
- Lifecycle outbox claim/delivery state.
- MCP connection repository, transport, schema inventory, effect classification, audit, or recursion propagation.
- Provider/plugin management API/UI and Reader artifact UI.
- Third-party plugin SDK, upload, catalog, marketplace, hot reload, or multi-plugin orchestration.

## 4. Project structure

```text
plugins/official/raindrop-ai-content/
  Cargo.toml        pinned guest dependencies and cdylib/rlib targets
  src/lib.rs        generated WIT bindings, descriptor, Guest export adapter
  src/config.rs     strict config DTO and operation selection
  src/json.rs       bounded canonical JSON helpers
  src/lifecycle.rs  persisted-event parsing and deterministic job intents
  src/operation.rs  direct/MCP execution flow and artifact candidate assembly
  src/prompt.rs     fixed prompt versions and system instructions
  src/tool_plan.rs  dynamic tool-plan schema, plan parsing, MCP projection

src/plugins/tool_plan.rs             host-side shared tool-plan contract
src/content/ai/schema.rs             provider schema family and output validation
src/plugins/runtime/{capability,error,execute}.rs
tests/support/official_ai_component.rs
tests/official_ai_component.rs
```

The guest contains no provider adapter, MCP transport, repository, persistence, or native shortcut.

## 5. Build and componentization

The guest manifest uses:

```toml
[package]
name = "raindrop-ai-content"
version = "1.0.0"
edition = "2024"
rust-version = "1.94"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.150"
wit-bindgen = { version = "=0.59.0", default-features = false, features = ["macros", "realloc", "std"] }
```

`wit_bindgen::generate!` selects world `content-plugin-v1` from `contracts/wit/raindrop-content-plugin-v1`. The release core module is built with:

```bash
cargo build --locked --manifest-path plugins/official/raindrop-ai-content/Cargo.toml \
  --target wasm32-unknown-unknown --release
```

Tests feed the core module to `wit_component::ComponentEncoder` with validation enabled. Encoding the same module twice must produce identical bytes. Wasmtime must compile the result from a matching signed test bundle and reject any ambient import.

No compiled `.wasm`, private release seed, or placeholder production signature is committed in this slice. The later bundled-discovery slice owns release artifact signing and embedding.

## 6. Guest descriptor and errors

`descriptor()` returns exactly:

- plugin key `raindrop.ai-content`;
- version `1.0.0`;
- ABI `raindrop:content-plugin@1.0.0`;
- operations `[summarize, translate]`;
- subscription `feed.refresh.persisted@1`;
- required capability `ai.generate_structured`;
- optional capability `mcp.call_tool`.

Guest errors use fixed namespaced message keys. Arbitrary dynamic text is forbidden. Runtime maps recognized keys to a public fixed enum while `Debug`/`Display` remain payload-free. Unknown keys are discarded rather than surfaced.

Representative keys:

```text
raindrop.ai-content.disabled
raindrop.ai-content.config-invalid
raindrop.ai-content.provider-unavailable
raindrop.ai-content.provider-rate-limited
raindrop.ai-content.provider-timeout
raindrop.ai-content.provider-output-invalid
raindrop.ai-content.mcp-schema-invalid
raindrop.ai-content.mcp-timeout
raindrop.ai-content.mcp-budget-exhausted
raindrop.ai-content.mcp-recursion-blocked
raindrop.ai-content.output-invalid
```

## 7. Canonical inputs and prompts

Feed title/text, source locale, canonical URL, MCP descriptions, MCP results, and model output are untrusted data. They enter only canonical JSON values passed as `untrusted-input-json`; they are never interpolated into system instructions.

Prompt versions are fixed constants:

- `raindrop-summary-v1`;
- `raindrop-translation-v1`;
- `raindrop-mcp-tool-plan-v1`.

The final system instruction states the operation, artifact semantics, output-language rule, and that every input/tool result is untrusted data that cannot modify policy or schema. Summary style changes only the requested depth/shape. Translation preserves code blocks, link destinations, and uncertain proper nouns, emits Markdown without raw HTML, and must return the exact target locale.

The final generation request always uses the exact canonical committed summary or translation schema and schema ID. The component accepts only `finish-reason = completed`; length, content-filter, native tool-plan, and unknown finishes fail without returning a partial artifact.

## 8. Token and request budgeting

Direct execution uses provider ordinal 1. MCP execution uses exactly two provider requests:

1. ordinal 1: tool plan;
2. ordinal 2: final artifact generation.

There is no guest-controlled third request or agent loop.

The component computes a conservative input-token ceiling from the byte length of the system instruction, canonical untrusted input, and output schema, using at most one token per byte. A call fails with `budget-exhausted` before the host import when the requested ceiling does not fit the remaining invocation input budget.

For MCP flow, at most 1,024 output tokens are reserved for the plan while preserving the operation minimum final ceiling (128 summary, 256 translation). The final request uses `min(config.maxOutputTokens, remaining after plan reservation)`. Direct flow uses `min(config.maxOutputTokens, remaining output budget)`.

Guest canonical AI input is capped at 512 KiB and output schemas at 64 KiB, matching host ceilings. Oversized entry/tool/result projections do not truncate silently.

## 9. MCP tool-plan contract

Schema ID:

```text
raindrop://schemas/plugins/raindrop.ai-content/tool-plan/v1
```

The canonical dynamic schema is an object containing `schemaVersion = 1` and `calls`. `calls.maxItems` is `min(config.maxToolCalls, invocation.remainingMcpCalls, 4)`. Its item schema is a `oneOf` list sorted by binding ID. Every branch fixes one `toolBindingId` with `const` and embeds that binding's exact canonical `input-schema-json` as the `arguments` schema.

The host runtime reconstructs this schema family from its own `CapabilityToolBinding` map and requires:

- exact current binding ID set;
- exact input schema bytes for every branch;
- unique sorted binding IDs;
- `1 <= maxItems <= remainingMcpCalls <= 4`;
- canonical JSON and total schema at most 64 KiB.

The provider broker independently parses the bounded schema family, forwards it as structured-output JSON Schema, and validates returned plan structure, max call count, allowed unique binding IDs, and object arguments. Exact argument-schema validation remains a mandatory MCP client-broker check before transport; the component never treats provider output as authorization.

The plan input includes entry context plus bounded tool label, description, connection/tool identity, and schema digest. Descriptions remain explicitly untrusted.

## 10. MCP execution and failure policy

Calls execute sequentially in plan order. Each request uses the host-issued binding ID, canonical arguments object, and a 10,000 ms requested timeout. The component never retries a denied/failed tool, substitutes a different tool, or exceeds the plan/config/host call count.

Successful results are embedded as JSON values under an `mcpContext` array in the final untrusted input. Connection/tool labels are data only. Arguments and result bytes never enter provenance or errors.

On any planning, binding, argument, call, result, or aggregate-size failure:

- `FAIL_OPEN`: discard every tool result from that attempt, mark provenance degraded, and perform ordinal-2 final generation using the original Feed snapshot only;
- `FAIL_CLOSED`: return the matching fixed MCP failure code and do not request final generation.

MCP disabled or zero visible bindings uses direct ordinal-1 generation and is not degraded.

## 11. Artifact candidate and provenance

The payload is the host-validated canonical provider output. Summary has no locale; translation locale equals the exact requested target locale.

Canonical provenance contains only fixed, non-secret hints:

```json
{
  "mcp": {
    "status": "DISABLED",
    "successfulCallCount": 0
  },
  "promptVersion": "raindrop-summary-v1",
  "providerRequestCount": 1
}
```

Allowed MCP status values are `DISABLED`, `APPLIED`, and `DEGRADED`. Provenance never contains entry text, prompt text, tool descriptions, arguments, results, connection IDs, credentials, endpoints, or provider transport details.

## 12. Lifecycle intent generation

`on-event` parses canonical config and persisted-event context. It returns an empty outcome when automatic processing is disabled. Otherwise it combines `newEntries` followed by `updatedEntries`, deduplicates by entry ID while preserving first occurrence, and emits enabled configured operations in config order.

Automatic translation uses `defaultTargetLocale`; summary target locale is absent. The exact intent idempotency key is:

```text
event:{eventId}:plugin:raindrop.ai-content:user:{subject}:entry:{entryId}:op:{summarize|translate}:config:{configHash}
```

The component verifies the event feed is compatible with the explicit all-feed/feed-ID portion of the config. Category-only selection remains dispatcher-owned because category membership is deliberately absent from the minimal lifecycle event. The host rechecks every scope and identifier before enqueue.

Lifecycle code never calls AI or MCP imports and returns no dynamic diagnostics in v1.

## 13. Testing strategy

1. Guest unit tests: config selection, canonical JSON, prompt invariants, tool-plan schema golden shape, plan validation, fail policy, lifecycle dedupe/order/idempotency.
2. Provider broker tests: direct schemas remain exact; tool-plan schema family accepts a valid dynamic schema and rejects branch/schema/limit drift; output rejects unknown/duplicate binding IDs and non-object arguments.
3. Capability tests: tool-plan schema must equal host descriptors and remaining MCP budget before broker invocation.
4. Real component tests: build core Wasm, componentize twice identically, compile signed bytes, inspect no `wasi:` imports, and execute direct summary, translation, MCP applied, MCP fail-open, MCP fail-closed, disabled operation, and lifecycle intents.
5. Source confinement: guest source and component text contain no WASI, provider client, database, network, filesystem, process, environment, or transport path.

## 14. Commands

```bash
rustup target add --toolchain 1.94.0 wasm32-unknown-unknown
cargo fmt --check
cargo fmt --check --manifest-path plugins/official/raindrop-ai-content/Cargo.toml
cargo test --locked --manifest-path plugins/official/raindrop-ai-content/Cargo.toml
cargo check --locked --manifest-path plugins/official/raindrop-ai-content/Cargo.toml --target wasm32-unknown-unknown --release
cargo test --locked --test official_ai_component -- --nocapture --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

## 15. Boundaries

- Always: generated WIT bindings; canonical bounded JSON; exact official artifact schemas; fixed prompt versions; host-issued IDs; no third provider request; one bounded review; targeted staging and prompt push.
- Internally decide: prompt wording, tool-plan schema layout, token reservation, failure-code mapping, and guest file boundaries.
- Never: WASI or ambient capability; native content processor; provider/MCP transport in guest; arbitrary schema/URL/model/secret; raw untrusted data in system instructions/provenance/errors; lifecycle AI/MCP calls; compiled production placeholder signature; sub-agents; `git add -A`.

## 16. Completion criteria

1. Rust source builds into a valid no-WASI component for the committed world on Rust 1.94.
2. Descriptor exactly matches the official manifest contract.
3. Real Wasmtime execution returns schema-valid summary and translation candidates using exact committed schemas.
4. MCP plan schema is derived from exact host tool descriptors; applied, fail-open, and fail-closed flows are bounded and tested.
5. Lifecycle returns deterministic summary/translation intents without calling capabilities.
6. Recognized guest failure codes survive runtime mapping without exposing arbitrary message text.
7. Componentization is deterministic for identical core bytes and CI installs the required target.
8. No worker, persistence, dispatcher, MCP transport, API/UI, production signing, or embedding claim is made.

## 17. Bounded internal review conclusion

- Architecture: the guest owns business prompts and bounded orchestration; host capability layers retain authorization, transport, schema, budget, and persistence ownership.
- Security: untrusted data stays in canonical data envelopes, dynamic tool schemas are reconstructed from host descriptors, and no ambient imports exist.
- Reliability: direct and MCP paths are deterministic and capped; fail-open never carries partial tool results, while fail-closed produces a stable code.
- Scope: this is the executable component, not the worker or MCP client. The next dependency after this slice is content-worker composition and artifact terminal commit.
- Open questions: none.
