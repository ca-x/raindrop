# Wasmtime Component Host Core v1 SDD Report

Date: 2026-07-19

Branch: `feature/foundation-bootstrap`

Binding specification: `docs/superpowers/specs/2026-07-19-wasmtime-component-host-core-v1-design.md`

Execution plan: `docs/superpowers/plans/2026-07-19-wasmtime-component-host-core-v1.md`

## Outcome

This slice adds the real Wasmtime Component Model host for verified bundled plugins. Raindrop can compile a signed component binary, instantiate it without ambient WASI, validate its descriptor, and call `execute` or `on-event` inside a fresh limited Store.

The host returns validated guest data only. It does not call `ProviderClient`, connect to an MCP transport, commit artifacts, dispatch lifecycle outbox records, or make summaries and translations visible in the Reader.

## Commits

- `a172194 docs: design wasmtime component host core`
- `9210ebf feat: generate plugin runtime bindings`
- `6a0a65a feat: compile verified plugin components`
- `fc35a6e feat: broker plugin host capabilities`
- `465af2a feat: execute sandboxed plugin components`
- `908bc18 test: keep plugin runtime lint clean`

## Delivered runtime

### Dependency and binding boundary

- Production uses exact `wasmtime = 46.0.1` with `async`, `call-hook`, `component-model`, `component-model-async`, `cranelift`, `runtime`, and `std` only.
- No `wasmtime-wasi`, WASI HTTP, cache, GC, threads, profiling, coredump, pooling allocator, or production WAT parser is present.
- Generated async and trappable bindings come from the committed `raindrop:content-plugin@1.0.0` WIT world. No second hand-written ABI exists.
- Tests use exact `wit-component` and `wasmprinter` 0.253.0 plus `wat` 1.253.0 tooling to build deterministic binary components. Production compilation accepts binary components through `Component::from_binary` only.

### Verified component compilation

- `CompiledPlugin::compile` requires a `BundledOfficialPlugin` that already passed manifest signature verification.
- Component input is capped at 16 MiB.
- The runtime recomputes SHA-256 and requires an exact match with the verified bundle digest before Wasmtime sees the bytes.
- Empty, oversized, malformed, WAT text, and digest-mismatched inputs fail with stable redacted error kinds.
- Compiled components are immutable and shareable. Store and instance state are never reused between invocations.

### Descriptor and link gate

- Each invocation builds a new linker containing only generated `host-ai` and `host-mcp` imports.
- Unknown imports fail as `LinkDenied`; no ambient import is stubbed.
- The guest descriptor must exactly match plugin key, version, ABI, operations, lifecycle subscription, and required/optional capabilities from the verified bundle contract.
- Capability access is suspended during instantiation and descriptor evaluation. AI and MCP imports remain denied until the descriptor matches, preventing an unverified component from spending broker budget before acceptance.
- Descriptor mismatch and descriptor trap stop execution before `execute` or `on-event`.

### Store sandbox

Every invocation receives a fresh Store with these limits:

- Linear memory: 64 MiB.
- Memories: 2.
- Tables: 4.
- Component instances: 4.
- Table elements: 10,000.
- Native Wasm stack: 512 KiB.
- Execute fuel: 50,000,000.
- Lifecycle fuel: 5,000,000.
- Cumulative pure guest CPU: 2 seconds.
- Operation/event request: 1 MiB.
- Entry text: 512 KiB.
- Config JSON: 256 KiB.
- Lifecycle context: 64 KiB.
- Artifact/event output: 512 KiB.

An independent 10 ms epoch thread interrupts CPU-bound guest code even when Tokio runs on one worker. The async call hook pauses guest CPU accounting while host code is running, so a 5 second broker wait does not consume the 2 second guest CPU budget.

### Capability session

- AI requests require the exact operation and provider binding, ordinal progression from 1, at most 3 calls, canonical object JSON, bounded system/input/schema data, remaining token budget, operation output ceiling, cost budget, and at most 90 seconds wall time.
- MCP requests require an exact tool binding, remaining depth, canonical arguments, at most 15 seconds wall time, and trigger-specific call limits: 2 for `FeedRefreshPersisted`, 4 for manual, Reader sidecar, or MCP server triggers.
- A session accepts at most 16 unique tool bindings.
- Feed automatic deadlines are capped at 120 seconds. Manual, Reader, and MCP server deadlines are capped at 180 seconds. Unix and monotonic deadlines must both be live and agree within 2 seconds.
- Broker responses are bounded and canonicalized before crossing the ABI. Debug, Display, and error output omit prompts, entry text, provider/tool results, credentials, endpoints, component bytes, and Wasmtime internals.
- `DenyMcpBroker` remains the default. This slice does not connect any MCP transport.

### Stable failures

The runtime distinguishes invalid component, digest mismatch, denied link, descriptor mismatch, invalid invocation, capability denial, broker timeout/failure, fuel exhaustion, memory limit, guest CPU timeout, generic guest trap, oversized output, and unavailable runtime.

## Main files

- `src/plugins/runtime/bindings.rs`
- `src/plugins/runtime/engine.rs`
- `src/plugins/runtime/component.rs`
- `src/plugins/runtime/capability.rs`
- `src/plugins/runtime/host.rs`
- `src/plugins/runtime/execute.rs`
- `src/plugins/runtime/error.rs`
- `tests/plugin_runtime_bindings.rs`
- `tests/plugin_runtime_component.rs`
- `tests/plugin_runtime_capabilities.rs`
- `tests/plugin_runtime_sandbox.rs`
- `tests/support/plugin_component.rs`

## Verification evidence

Final local commands:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test plugin_runtime_bindings --test plugin_runtime_component --test plugin_runtime_capabilities --test plugin_runtime_sandbox
cargo test --all-targets
cargo +1.94.0 test --test plugin_runtime_sandbox
```

Results:

- Formatting: pass.
- Clippy: pass with warnings denied.
- Runtime integration tests: 19 passed, 0 failed.
- Runtime unit tests: 3 passed, 0 failed.
- Rust 1.94 sandbox tests: 5 passed, 0 failed.
- Full tests discovered: 583.
- Full tests passed: 582.
- Full tests failed: 0.
- Full tests ignored: 1.
- The ignored test is `ithome_feed_securely_ingests_and_deduplicates`, which requires `RAINDROP_LIVE_RSS_SMOKE=1` and public network.
- The existing SeaORM dependency chain warning for `proc-macro-error2 2.0.1` remains tracked in `tasks/todo.md` and was not introduced by this slice.

CI run `29663971184` passed all seven jobs: Rust service databases and full tests, Windows durable replacement, current-stable compatibility, ASTRYX Web, supply-chain audit, release embedding and E2E, and the non-root container build and health smoke.

## Explicitly still pending

- The release-signed production `raindrop.ai-content.wasm` component.
- ProviderClient broker composition, quota/cost reservation, prompt/schema execution, and summary/translation artifact completion.
- Provider management API/UI and content execution/retry routes.
- Feed lifecycle dispatcher, durable delivery, outbox retry, and circuit breaker.
- External MCP Streamable HTTP/limited stdio client and tool execution.
- Raindrop MCP Streamable HTTP/stdio server, scopes, tokens, and user isolation.
- AI artifact Reader sidecar and plugin management UI.
- Third-party component installation, SDK publication, marketplace, and hot reload.

## Next dependency

The next implementation slice is Official AI Component / ProviderClient Broker Composition v1:

1. build and release-sign `raindrop.ai-content.wasm` against the committed WIT;
2. adapt the host AI capability to `ProviderClient` without exposing credentials or transport details;
3. reserve quota, token, and cost budget around each fenced content-job attempt;
4. execute summary and translation prompts with strict output schemas;
5. commit validated artifacts only through the existing content repository terminal operations.
