# Wasmtime Component Host Core v1 Design

Date: 2026-07-19

Status: binding implementation slice

Parent specifications:

- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md`

## 1. Objective

Deliver the real Wasmtime Component Model host that can compile and instantiate only a previously verified bundled plugin, link only the committed `host-ai` and `host-mcp` imports, enforce the v1 sandbox limits, validate the guest descriptor, and invoke `execute` or `on-event` through generated typed bindings.

This slice also delivers the invocation-scoped capability session used by host imports. It validates binding IDs, operation identity, canonical JSON, token/call/depth budgets, timeouts, and broker outputs before data crosses the ABI.

It does not call `ProviderClient`, execute an official summary/translation component, commit an artifact, dispatch lifecycle outbox records, or connect MCP transport. Those operations require the next AI Execution Broker / Official Component slice and must reuse this host rather than create a native processor.

## 2. Assumptions and delegated review

- The user delegated review/confirmation to the main Agent and requested no sub-agent development.
- The committed WIT package `raindrop:content-plugin@1.0.0` is authoritative.
- `BundledOfficialPlugin` remains the only constructor proving release signature and component digest origin.
- `ContentJobClaim`, `ProviderClient`, `ContentRepository`, and Plugin Registry remain separate records/services; Wasmtime state is transient.
- Rust remains edition 2024 with MSRV 1.94.

## 3. Dependency decision

Use exact `wasmtime = 46.0.1`, whose declared MSRV is Rust 1.94.0. Disable default features and enable only:

- `async`
- `call-hook`
- `component-model`
- `component-model-async`
- `cranelift`
- `runtime`
- `std`

Do not add `wasmtime-wasi`, `wasmtime-wasi-http`, `cache`, `wat`, pooling allocator, GC, threads, profiling, coredump, or debug symbol features.

Tests use `wit-component = 0.253.0` with `dummy-module`, `wasmprinter = 0.253.0`, and `wat = 1.253.0` only to synthesize deterministic component fixtures from the committed WIT. The fixture builder prints the generated core module, applies bounded mechanical replacements, and parses the result back to binary. Production code never accepts WAT text and uses `Component::from_binary`.

## 4. Scope

### 4.1 Included

1. Generated async host/guest bindings from `contracts/wit/raindrop-content-plugin-v1`.
2. A single hardened `Engine` policy and 10 ms epoch ticker.
3. `CompiledPlugin::compile` requiring both `BundledOfficialPlugin` and matching component bytes.
4. Fresh Store and component instance per invocation.
5. Descriptor check against verified bundle identity before `execute` or `on-event`.
6. Exact sandbox limits:
   - 64 MiB linear memory hard limit;
   - at most 2 memories, 4 tables, 4 instances, and 10,000 table elements;
   - 512 KiB native Wasm stack;
   - 50,000,000 fuel for `execute`;
   - 5,000,000 fuel for lifecycle callback;
   - 2 seconds cumulative pure guest CPU time;
   - 1 MiB operation/event request;
   - 512 KiB artifact candidate/event outcome;
   - 16 MiB component binary.
7. Invocation-scoped AI/MCP capability session with async broker traits and deny-by-default implementations.
8. Stable redacted runtime error kinds for compile/link/descriptor/fuel/memory/timeout/trap/output/broker failures.
9. Deterministic fixture generation and real Wasmtime tests for compile, link denial, resource limits, fuel, epoch, and trap classification.

### 4.2 Excluded

- `ProviderClient` adapter, provider repository loading, rate/concurrency/quota/cost reservation, or real model request.
- Official `raindrop.ai-content.wasm` business logic, prompts, summary, translation, or artifact commit.
- MCP client connection, discovery, credentials, Streamable HTTP, stdio, or tool execution.
- Lifecycle dispatcher/delivery persistence or Feed transaction changes.
- Plugin HTTP API/UI, arbitrary component upload, marketplace, filesystem discovery, or hot reload.
- WASI preview1/preview2, sockets, filesystem, environment, process, clocks, random, or inherited stdio.

## 5. Module topology

```text
src/plugins/runtime/
  mod.rs          public host facade and limits
  bindings.rs     generated WIT bindings only
  engine.rs       hardened Engine + epoch ticker
  component.rs    verified binary compilation
  capability.rs   invocation-scoped AI/MCP validation and brokers
  host.rs         generated host trait implementations
  execute.rs      Store/Linker/descriptor/execute/on-event flow
  error.rs        stable redacted error contract
```

The existing `src/plugins` manifest/config/registry modules remain unchanged in ownership. The runtime may read `BundledOfficialPlugin` fields but never reads the database directly.

## 6. Engine and sandbox policy

The engine enables Component Model, async support, fuel, and epoch interruption. It disables unrelated optional Wasmtime features at Cargo resolution.

Each Store contains:

- `StoreLimits` with the fixed memory/table/instance bounds and trap-on-grow-failure;
- one invocation capability session;
- a guest CPU budget tracker;
- no WASI context, resource table, socket handle, repository, secret, or environment map.

The linker is newly created per invocation and receives only generated `host-ai` and `host-mcp` bindings. Any additional import causes link failure. No ambient import is stubbed or silently ignored.

The capability session is suspended during instantiation and `descriptor`. Host AI and MCP imports return capability denial until the descriptor exactly matches the verified bundle. The runtime activates the session only before `execute` or `on-event`, so an unverified descriptor cannot spend provider or MCP budget.

## 7. Pure guest CPU deadline

A shared ticker increments the engine epoch every 10 ms. Every invocation starts with 2 seconds of cumulative guest time.

The Wasmtime async call hook observes transitions:

- `CallingWasm` / `ReturningFromHost`: start measuring guest time and set the epoch deadline to the ceiling of remaining time / 10 ms.
- `CallingHost` / `ReturningFromWasm`: subtract elapsed guest time and pause measurement.

Host broker await time therefore does not consume the 2-second guest CPU budget. AI and MCP calls have their own wall-clock timeout. If remaining guest time reaches zero, the call hook or epoch trap maps to `GuestTimeout`.

Fuel remains deterministic and independent. An invocation can fail with `FuelExhausted` before reaching the epoch deadline.

## 8. Verified compilation

`CompiledPlugin::compile(runtime, bundle, component_bytes)`:

1. rejects empty or over-16-MiB input;
2. recomputes SHA-256 and requires equality with `bundle.component_digest()`;
3. uses `Component::from_binary`, never WAT parsing or unsafe deserialization;
4. stores only the compiled component and verified descriptor identity;
5. redacts Wasmtime parser/compiler messages from the public error.

Compiled components are immutable and shareable. Store/instance state is never reused between invocations.

## 9. Descriptor and invocation contract

Before any business call, the host invokes `descriptor` and requires exact equality for:

- plugin key;
- plugin version;
- ABI;
- operations `[summarize, translate]`;
- lifecycle subscription `feed.refresh.persisted@1`;
- required capability `ai.generate_structured`;
- optional capability `mcp.call_tool`.

Mismatch returns `DescriptorMismatch`. A descriptor trap is a guest failure and cannot be bypassed by trusting the manifest alone.

The descriptor phase cannot call a capability broker. This is enforced in host state rather than assumed from the WIT signature.

`execute` accepts only the generated WIT operation request after a domain constructor checks:

- total canonical request at most 1 MiB;
- sanitized entry text at most 512 KiB;
- verified plugin key/version/digest equal the compiled component;
- operation/target locale/artifact schema consistency;
- config JSON is canonical and its hash matches;
- provider/tool binding IDs and call-chain/depth/budget values are bounded;
- invocation deadline does not exceed the enclosing job attempt deadline.

`on-event` accepts the already validated `LifecycleEvent` contract and gets lifecycle fuel, not execute fuel.

The host validates returned artifact/event JSON and size. It returns data only; it never writes the artifact or enqueues jobs in this slice.

## 10. Capability session

The generated host traits delegate to one `CapabilitySession` owned by the Store.

### 10.1 AI

The session requires:

- exact current operation;
- exact host-issued provider binding ID;
- monotonically increasing provider request ordinal starting at 1;
- no more than 3 calls;
- canonical object input and output schema;
- input at most 512 KiB and schema at most 64 KiB;
- system instruction at most 64 KiB;
- requested input tokens no greater than the invocation remaining-input budget;
- requested output tokens no greater than both the invocation remaining-output budget and the operation hard limit: 4,096 for `summarize`, 16,384 for `translate`;
- broker wall timeout no greater than 90 seconds.

The broker returns validated object JSON, normalized finish reason, optional usage/cost, and a bounded public model label. Broker errors are converted to the committed WIT error family without including provider body, prompt, schema, credential, or endpoint.

### 10.2 MCP

The session requires an exact host-issued tool binding ID, remaining depth, trigger-specific call count, canonical object arguments at most 64 KiB, and timeout at most 15 seconds. `FEED_REFRESH_PERSISTED` permits at most 2 calls; `MANUAL_API`, `READER_SIDECAR`, and `MCP_SERVER` permit at most 4. Results are canonical JSON at most 256 KiB.

The default runtime is constructed with `DenyMcpBroker`; real MCP remains impossible until an explicit later broker is injected.

### 10.3 Broker interfaces

```rust
#[async_trait]
pub trait AiCapabilityBroker: Send + Sync {
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError>;
}

#[async_trait]
pub trait McpCapabilityBroker: Send + Sync {
    async fn call_tool(
        &self,
        context: &BrokerInvocationContext,
        request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError>;
}
```

These are capability interfaces, not provider/MCP transports. The next slice implements the ProviderClient adapter only after quota/cost reservation is designed.

## 11. Error mapping

| Runtime kind | Meaning |
| --- | --- |
| `InvalidComponent` | malformed/non-component binary or component over size limit |
| `ComponentDigestMismatch` | bytes differ from verified bundle digest |
| `LinkDenied` | unknown/ambient import or incompatible ABI |
| `DescriptorMismatch` | guest descriptor differs from signed manifest contract |
| `InvalidInvocation` | request/config/hash/budget/JSON contract invalid |
| `CapabilityDenied` | provider/tool binding or operation not granted |
| `BrokerTimeout` | AI/MCP broker exceeded its wall timeout |
| `BrokerFailure` | safe broker error not represented by guest result |
| `FuelExhausted` | deterministic fuel depleted |
| `MemoryLimit` | memory/table/instance/growth limit denied |
| `GuestTimeout` | cumulative guest CPU epoch exhausted |
| `GuestTrap` | other guest trap |
| `OutputTooLarge` | artifact/event/broker output exceeds bound |
| `RuntimeUnavailable` | no Tokio runtime or epoch ticker cannot start |

`Debug`, `Display`, and error chains never expose component bytes, WIT payloads, entry content, config, prompts, provider/tool results, identifiers, or Wasmtime internal messages.

## 12. Tests

Approved seams:

1. `PluginRuntime::new` and `CompiledPlugin::compile` for dependency/engine/digest/binary bounds.
2. `CapabilitySession` with mock brokers for authorization, counters, canonical JSON, timeout, output validation, and redaction.
3. `PluginRuntime::{execute,on_event}` with generated dummy components for link denial, descriptor trap, fuel exhaustion, epoch timeout, and memory limit.
4. Source confinement test proving no `wasmtime_wasi`, `WasiCtx`, socket, filesystem, environment, process, or direct `ProviderClient` use inside runtime modules.

Fixture generation uses committed WIT plus `wit-component::dummy_module`, embeds component metadata, and encodes a validated component. Hostile variants are generated only in tests by printing and mechanically replacing deterministic dummy-module bodies/memory declarations.

## 13. Boundaries

- Always: verified bundle type before compile; fresh Store; exact linker imports; descriptor before business call; validate all WIT/broker inputs and outputs; use fuel and epoch; push each verified commit.
- Internally review once: Wasmtime features, generated interface, call-hook CPU accounting, limits, error classification, and broker surface.
- Never: add WASI; deserialize untrusted compiled artifacts; expose raw Wasmtime errors; call ProviderClient directly; commit an artifact; execute in Feed transaction; accept arbitrary upload; edit `.superpowers/research/` or root `node_modules/`; use sub-agents; use `git add -A`.

## 14. Completion criteria

1. Wasmtime 46.0.1 compiles on Rust 1.94 with only the listed features.
2. Bindings generate from committed WIT without a second hand-written ABI.
3. Matching signed bundle/component compiles; mismatched, malformed, WAT, and oversized inputs fail closed.
4. Unexpected imports fail link because no ambient linker capability exists.
5. Memory above 64 MiB, fuel exhaustion, cumulative guest CPU timeout, and generic traps map to distinct stable errors.
6. AI/MCP capability sessions enforce exact binding, operation, call count, token/depth/time/JSON/output bounds and never echo sensitive payloads.
7. A descriptor is required before execute/on-event and mismatch cannot proceed.
8. No ProviderClient/MCP transport/database/artifact commit/lifecycle dispatch claim is added.
9. Format, Clippy, targeted tests, full Rust tests, Windows compile, release E2E, and non-root container CI pass.

## 15. Bounded internal review conclusion

- DDIA: compiled components and Store state remain transient; database records and content jobs stay authoritative.
- Security: verified origin, binary-only compilation, no WASI, exact imports, fresh Store, limits, fuel, paused host-time epoch accounting, and redacted errors address the execution boundary.
- API design: generated WIT bindings prevent a second ABI; broker traits are narrow capabilities rather than transports; runtime returns data without taking persistence ownership.
- Scope: this is a real sandbox host, not a fake processor. It intentionally leaves provider quota/cost composition and the official business component for the next vertical slice.
