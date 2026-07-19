# Official AI Component Invocation Contract v1 Design

Date: 2026-07-19

Status: internally approved pre-component contract repair

Parent specifications:

- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/superpowers/specs/2026-07-19-official-ai-plugin-contract-registry-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-wasmtime-component-host-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-ai-provider-broker-composition-v1-design.md`

## 1. Objective

Repair the still-unreleased WIT v1 invocation contract so the official `raindrop.ai-content` component can safely implement both configured lifecycle job intents and schema-aware MCP context enrichment without receiving credentials, transports, repositories, or ambient WASI.

Two concrete gaps block a correct component today:

1. `tool-binding` contains only `binding-id` and `display-label`. The guest cannot correlate it to configured `(connectionId, toolName)`, inspect the host-validated input schema, or construct a schema-constrained tool plan.
2. `content-plugin.on-event` receives only the lifecycle event. The guest cannot see the canonical per-user plugin config snapshot needed to decide whether automatic summarize/translate operations are enabled.

The plugin ABI has not shipped in `v0.1.0`; that release contains no executable plugin. Correcting `raindrop:content-plugin@1.0.0` now avoids freezing an unusable observable contract. Once an executable component ships, future incompatible changes require a new WIT major.

## 2. Scope

### 2.1 Included

1. Enrich WIT `tool-binding` with exact non-secret planning metadata:
   - host-issued binding ID;
   - connection UUID;
   - exact tool name;
   - bounded display label;
   - bounded untrusted description;
   - canonical input JSON Schema;
   - lower-hex schema digest.
2. Add WIT `lifecycle-request` containing host invocation identity, verified plugin identity, canonical config snapshot/hash, and one lifecycle event.
3. Change `content-plugin.on-event` to consume `lifecycle-request`.
4. Replace capability-session ID-only tool configuration with typed `CapabilityToolBinding` descriptors.
5. Validate every descriptor field, canonical schema, digest, uniqueness, count, and exact equality between host session and WIT operation request.
6. Validate lifecycle request identity/config/event before invoking the guest.
7. Keep capability imports suspended for descriptor and lifecycle calls. Only `execute` activates AI/MCP capabilities.
8. Update generated bindings, runtime size accounting, dummy component fixtures, WIT contract tests, and source-confinement tests.
9. Update the parent specifications to record the corrected unshipped v1 contract.

### 2.2 Excluded

- Official component Rust source or compiled `.wasm`.
- MCP connection storage, discovery, transport, credential, tool execution, output-schema validation, or audit.
- Provider/API/UI/content worker/artifact/lifecycle dispatcher implementation.
- Arbitrary plugin upload or third-party distribution.
- A compatibility shim for the pre-release broken WIT shape; no executable release consumed it.

## 3. Corrected WIT contract

### 3.1 Tool binding

```wit
record tool-binding {
  binding-id: string,
  connection-id: string,
  tool-name: string,
  display-label: string,
  description: string,
  input-schema-json: canonical-json,
  input-schema-digest: string,
}
```

Rules:

- `binding-id`: host-issued visible ASCII, 1..=128 bytes, unique per invocation.
- `connection-id`: canonical UUID matching the user-owned configured connection.
- `tool-name`: exact configured name, 1..=128 visible ASCII bytes, first alphanumeric, remaining `[A-Za-z0-9._:/-]`.
- `display-label`: 1..=128 UTF-8 bytes, no control characters.
- `description`: untrusted text, at most 8 KiB, line controls allowed; it is data for the model, never policy.
- `input-schema-json`: canonical JSON object, at most 64 KiB, already validated by MCP client core against the current tool inventory.
- `input-schema-digest`: lower-hex BLAKE3 over a domain-separated frame of the canonical schema.
- At most 16 tool bindings; `(connection-id, tool-name)` and `binding-id` are independently unique.
- The host session stores the complete descriptor. The WIT request must equal it byte-for-byte; the guest cannot substitute a schema, description, name, or connection.

The binding contains no endpoint, credential, auth header, stdio command, socket, transport object, or connection pool.

### 3.2 Lifecycle request

```wit
record lifecycle-request {
  invocation-id: string,
  plugin-key: string,
  plugin-version: string,
  component-digest: string,
  config-json: canonical-json,
  config-hash: string,
  event: lifecycle-event,
}
```

`content-plugin.on-event` becomes:

```wit
on-event: func(request: lifecycle-request) -> result<event-outcome, plugin-error>;
```

The wrapper is deliberately smaller than `operation-request`:

- lifecycle delivery creates declarative job intents only;
- it has no provider binding, tool binding, job ID, or model/MCP budget;
- the event already contains the opaque user scope and stable event identity;
- execution fuel/CPU/output limits remain host-owned runtime policy.

The host validates:

- invocation ID is bounded visible ASCII;
- plugin key/version/digest equal the verified compiled component;
- config parses as `AiContentConfig`, is canonical, and its hash matches;
- event passes the existing versioned `LifecycleEvent` parser;
- event user subject equals the suspended capability session subject;
- event type is exactly a descriptor-declared subscription.

## 4. Runtime domain types

```rust
pub struct CapabilityToolBinding {
    binding_id: String,
    connection_id: String,
    tool_name: String,
    display_label: String,
    description: String,
    input_schema_json: String,
    input_schema_digest: String,
}

impl CapabilityToolBinding {
    pub fn new(input: CapabilityToolBindingInput) -> Result<Self, PluginRuntimeError>;
    pub fn to_wit(&self) -> types::ToolBinding;
}
```

`CapabilitySessionConfig` replaces `tool_binding_ids: Vec<String>` with `tool_bindings: Vec<CapabilityToolBinding>`. `CapabilitySession` keeps a map keyed by binding ID so `host-mcp.call-tool` authorization remains O(log n) and can later hand the complete descriptor to MCP client core without trusting guest-supplied metadata.

The public type uses custom `Debug` that reports only binding count/byte sizes and never description or schema contents.

## 5. Capability activation

The runtime state transitions are corrected to:

| Phase | AI/MCP imports |
| --- | --- |
| instantiate | denied |
| descriptor | denied |
| execute | activated after descriptor and request validation |
| on-event | denied for the entire invocation |

Lifecycle callbacks therefore cannot call a provider or MCP even if a malicious component imports the functions. The official component only converts committed event/config data into bounded declarative intents; all later entry visibility/provider/automatic-scope checks occur again in the dispatcher/orchestrator.

## 6. Validation and size limits

- Operation request remains at most 1 MiB.
- Each tool schema is at most 64 KiB and each description at most 8 KiB; all descriptor bytes participate in operation request size accounting.
- Lifecycle request remains at most 1 MiB; event context remains at most 64 KiB and config at most 256 KiB.
- Schema digest uses `blake3::Hasher::new_derive_key("raindrop.mcp-tool-input-schema.v1")` plus framed canonical schema bytes.
- Duplicate JSON keys, non-object schemas, non-canonical schema/config, digest mismatch, descriptor drift, and unknown event subscription fail as `InvalidInvocation` before guest business execution.
- Errors and `Debug` never include schema, description, config, event context, Feed text, or tool metadata contents.

## 7. Testing strategy

1. WIT parser selects the corrected world and asserts the enriched record/lifecycle signature through generated Rust types rather than text-only matching.
2. Capability tests accept a complete valid descriptor and reject:
   - duplicate binding ID;
   - duplicate connection/tool pair;
   - invalid UUID/tool name;
   - control/oversized label or description;
   - array/non-canonical/oversized schema;
   - schema digest mismatch;
   - operation request field drift.
3. Runtime tests prove `on-event` receives config, validates plugin/config/event identity, returns the fixture outcome, and cannot call AI/MCP while suspended.
4. Request size tests include all new descriptor/config fields and enforce exact N/N+1 boundaries.
5. Source confinement still rejects WASI, sockets, process, provider client, MCP transport, repositories, and database types in `src/plugins/runtime`.

## 8. Commands

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test plugin_runtime_bindings -- --nocapture
cargo test --locked --test plugin_runtime_capabilities -- --nocapture
cargo test --locked --test plugin_runtime_sandbox -- --nocapture
cargo test --locked --test plugin_contract_assets -- --nocapture
cargo test --locked --all-features
git diff --check
```

## 9. Boundaries

- Always: generated binding is authoritative; complete host-issued tool descriptor equality; canonical schema/config; exact digest; lifecycle config snapshot; suspended lifecycle capabilities; bounded redacted errors.
- Internally review once: WIT record/signature, descriptor limits, digest framing, lifecycle wrapper, and activation state.
- Never: pass credential/endpoint/transport through WIT; let guest metadata authorize a tool; call provider/MCP during lifecycle; add compatibility code for an unshipped shape; claim component/MCP/dispatcher completion; use sub-agents; use `git add -A`.

## 10. Completion criteria

1. Corrected WIT v1 exposes enough non-secret data for a schema-aware tool plan and configured lifecycle intent generation.
2. Generated bindings and dummy components compile against the corrected world.
3. Host session owns complete tool descriptors and rejects every request drift before broker invocation.
4. Lifecycle request carries canonical config and verified plugin identity and passes the existing event contract.
5. AI/MCP imports remain denied for descriptor and lifecycle; only execute activates them.
6. Full Rust gates pass and no executable component/MCP/dispatcher claim is made.

## 11. Bounded internal review conclusion

- DDIA: lifecycle events/config remain database-derived facts; Wasm receives snapshots and returns intents, not authoritative writes. Schema digests identify an observed tool contract without making runtime memory a record system.
- API/interface: the corrected record supplies the minimum data required for safe planning and exact config semantics; it still hides transports and secrets. Because v1 is unshipped, correcting it now is safer than freezing an unusable interface and adding compensating APIs.
- Security: descriptions and schemas are explicitly untrusted, authorization remains binding-ID based and host-owned, schema drift fails closed, and lifecycle cannot spend AI/MCP capability.
- Scope: this slice repairs the invocation contract only. Official component, MCP client, dispatcher, worker, artifacts, and UI remain subsequent vertical slices.
- Open questions: none. The official component can be designed immediately after this corrected contract passes.
