# AI Provider Broker Composition v1 Design

Date: 2026-07-19

Status: internally approved implementation slice

Parent specifications:

- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/superpowers/specs/2026-07-18-ai-provider-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-content-jobs-artifacts-core-v1-design.md`
- `docs/superpowers/specs/2026-07-19-wasmtime-component-host-core-v1-design.md`

## 1. Objective

Connect the existing Wasmtime `AiCapabilityBroker` boundary to the real `ProviderRepository` and `ProviderClient` without moving provider secrets, transport objects, database connections, or provider DTOs through WIT.

Success means an invocation-scoped host request can load only an enabled provider visible to the requesting user, enforce provider and invocation limits before network I/O, execute exactly one of the four committed provider adapters, validate the returned object against the exact official summary or translation schema, and return a canonical redacted broker response to the capability session.

This is an internal execution slice. It does not yet build or embed `raindrop.ai-content.wasm`, claim a content job, commit an artifact, expose provider/plugin HTTP APIs, dispatch lifecycle events, or connect MCP transport.

## 2. Assumptions and delegated review

- The user delegated confirmation and requested that the main Agent continue without sub-agents, so this specification receives one bounded internal review and then proceeds directly to implementation.
- Raindrop v1 is one server process. Provider concurrency and sliding-minute admission are process-local runtime controls. Durable jobs and artifacts remain database-authoritative; multi-process distributed provider admission requires a later explicit coordination design rather than pretending an in-memory semaphore is cross-node consensus.
- `ProviderRepository::load_enabled_binding` is the only path that may decrypt a provider credential. The broker never persists or formats the binding.
- `CapabilitySession` remains responsible for invocation call-count, remaining token, remaining cost, deadline, and operation checks. The broker independently rechecks provider policy and official schema identity at its trust boundary.
- A provider call idempotency key is host-derived from the content job ID and provider request ordinal. It does not include the attempt number, so retrying an unknown outcome converges on the same upstream operation when the provider supports idempotency.
- Official component and MCP work follow this slice. The current WIT `tool-binding` exposes only binding ID and display label, not the validated tool input schema needed for a safe general tool plan. That interface gap must be resolved before the official component claims MCP enrichment.

## 3. Scope

### 3.1 Included

1. A production `ProviderAiBroker<T: ProviderTransport>` implementing `AiCapabilityBroker`.
2. Exact user/provider visibility through `ProviderRepository::load_enabled_binding`.
3. Provider policy enforcement before transport:
   - maximum input tokens;
   - maximum output tokens;
   - per-provider maximum concurrency;
   - optional sliding 60-second request limit;
   - conservative request cost estimation and invocation/provider cost ceilings.
4. Stable host-derived provider idempotency keys.
5. Exact operation-to-schema registry for `ai-summary/v1` and `ai-translation/v1`.
6. Typed validation and canonicalization of provider output through `SummaryArtifact` and `TranslationArtifact`.
7. Translation target-locale equality across request input, broker output, artifact locale, and final runtime validation.
8. Stable mapping from provider repository/client failures into `AiBrokerError` without payload, endpoint, credential, prompt, schema, or model output disclosure.
9. Deterministic SQLite-plus-fake-transport tests covering all four provider kinds, admission, schema rejection, locale mismatch, idempotency, cost, timeout/error mapping, and redaction.

### 3.2 Excluded

- Official guest component source, componentization, release signing, embedding, or plugin installation sync.
- Content worker claim/heartbeat/completion orchestration or attempt usage persistence.
- Provider administration API/UI or live credential probe.
- MCP connection, tool schema inventory, tool broker, audit, or recursion transport.
- Cross-process/distributed provider rate or concurrency coordination.
- New database tables or a second content-job/provider record system.

## 4. Commands

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test ai_provider_broker -- --nocapture --test-threads=1
cargo test --locked --test plugin_runtime_capabilities -- --nocapture
cargo test --locked --test plugin_runtime_sandbox -- --nocapture
cargo test --locked --all-features
git diff --check
```

No deterministic test sends a real provider request or contains a live credential.

## 5. Module topology

```text
src/content/ai/
  mod.rs          public broker facade
  admission.rs    process-local provider concurrency and sliding-minute admission
  broker.rs       repository + client composition and safe error mapping
  cost.rs         checked conservative/actual cost estimation
  schema.rs       exact official schema registry and typed output validation

src/plugins/runtime/capability.rs
  host-issued job identity, per-call cost ceiling, and broker error accessors

src/plugins/runtime/execute.rs
  translation payload target-locale invariant

tests/ai_provider_broker.rs
  full broker composition contract with fake provider transport
```

`src/plugins/runtime` continues to contain no `ProviderClient`, repository, database, or transport reference. Composition lives in `src/content/ai`, outside the sandbox host.

## 6. Interface contract

```rust
pub struct ProviderAiBroker<T> {
    repository: Arc<ProviderRepository>,
    client: Arc<ProviderClient<T>>,
    admission: Arc<ProviderAdmissionController>,
}

impl<T: ProviderTransport> ProviderAiBroker<T> {
    pub fn new(
        repository: Arc<ProviderRepository>,
        client: Arc<ProviderClient<T>>,
    ) -> Self;
}

#[async_trait]
impl<T: ProviderTransport> AiCapabilityBroker for ProviderAiBroker<T> {
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError>;
}
```

`BrokerInvocationContext` gains the host-issued `job_id`. `CapabilitySession` validates the WIT operation request job ID against that value. The field is excluded from `Debug`.

`AiBrokerRequest` gains `max_cost_micros`, copied from the invocation remaining-cost budget after the capability session has validated the request. The guest cannot select or increase this value because it is not present in WIT.

## 7. Execution flow

1. Validate that the requested schema ID is exact for the operation and that the supplied canonical schema equals the committed contract schema.
2. Load the enabled binding by `(provider_binding_id, context.user_subject)`.
3. Recheck provider input/output token policy.
4. Compute the conservative maximum cost from requested input/output token ceilings when both provider rates are known. Reject when it exceeds either the invocation cost ceiling or provider `max_cost_micros_per_request`.
5. Acquire the process-local provider revision admission state:
   - non-blocking concurrency permit;
   - sliding 60-second request reservation.
6. Derive the provider idempotency key from framed BLAKE3 over `job_id` and `provider_request_ordinal`.
7. Build the canonical `StructuredGenerationRequest` using the binding model and exact official schema name.
8. Execute `ProviderClient::generate` once. The capability session supplies the enclosing wall timeout; the provider transport retains its staged DNS/connect/header/body/total deadlines.
9. Validate the returned object with the exact artifact type. For translation, require returned `targetLocale` to equal the canonical target locale in untrusted input.
10. Return canonical object JSON, normalized finish reason, bounded public model label, usage, and conservative actual cost estimate.

The broker never writes a job/artifact and never opens a database transaction around network I/O.

## 8. Admission and cost semantics

Admission state is keyed by provider ID plus provider revision. A policy update creates a new state, so a changed concurrency/rate contract cannot mutate an in-flight semaphore.

- Concurrency uses `try_acquire_owned`; saturation returns retryable `RateLimited` without queueing unbounded broker futures.
- Requests-per-minute uses a monotonic sliding 60-second window. A reservation is recorded immediately before the provider request and counts failed/timeout requests because the upstream request consumed capacity.
- State is operational and process-local. Restart clears the window; database jobs/artifacts and idempotency still preserve correctness.
- When both input and output prices are present, maximum cost is calculated with checked `u128` multiplication and ceiling division by 1,000,000.
- When response usage is complete, returned estimated cost uses actual usage. Otherwise it uses the admitted conservative maximum.
- When pricing is incomplete, cost is `None`; token, request, concurrency, and invocation call-count limits still apply.

## 9. Schema and output contract

Allowed pairs are exact:

| Operation | Schema ID | Provider schema name |
| --- | --- | --- |
| summarize | `raindrop://schemas/artifacts/ai-summary/v1` | `raindrop_ai_summary_v1` |
| translate | `raindrop://schemas/artifacts/ai-translation/v1` | `raindrop_ai_translation_v1` |

The requested schema document is parsed, canonicalized, and compared to the committed JSON file. An alternative schema with the same ID, unknown fields, duplicate keys, or reordered non-canonical request text fails before provider I/O.

Provider output is untrusted. Summary output must pass `SummaryArtifact::parse`; translation output must pass `TranslationArtifact::parse` and exact target-locale equality. The canonical typed artifact JSON is what returns to Wasm.

## 10. Error mapping

| Source | Broker error | Retryable |
| --- | --- | --- |
| invisible/invalid binding or user | `CapabilityDenied` | no |
| disabled/corrupt/secret unavailable provider | `ProviderUnavailable` | no |
| provider repository database failure | `ProviderUnavailable` | yes |
| input/output/provider cost policy | `QuotaExceeded` / `CostLimitExceeded` | no |
| local concurrency/RPM or upstream 429 | `RateLimited` | yes |
| transport/provider timeout | `Timeout` | yes |
| malformed/oversized/schema-invalid output | `OutputSchemaInvalid` | no |
| invalid adapter request or rejected request | `InvalidRequest` | no |
| network/upstream failure | `ProviderUnavailable` | yes |

`Debug`, `Display`, and error chains expose only stable kinds, retryability, optional retry time, operation, and trigger. Tests use sentinels to prove provider ID, endpoint, credential, prompt, input, schema, and output do not escape.

## 11. Testing strategy

- One SQLite repository fixture creates a real encrypted provider binding and active user.
- A fake transport records only expected adapter properties inside assertions and returns provider-specific response envelopes.
- The four provider kinds each execute once and return the same canonical summary artifact.
- Request bodies prove the exact provider adapter selected; idempotency is identical for the same job/ordinal and differs by ordinal/job.
- Wrong user, disabled provider, input/output limit, cost ceiling, concurrency saturation, RPM exhaustion, invalid schema, malformed output, translation locale mismatch, 429, timeout, and upstream errors map exactly.
- Source confinement continues proving `src/plugins/runtime` contains no provider/database/transport shortcut.

## 12. Boundaries

- Always: exact user scope; exact operation/schema pair; short repository reads; no transaction across network; conservative preflight; provider output treated as hostile; stable host-derived idempotency; bounded redacted errors.
- Internally review once: broker interface additions, process-local admission limitation, cost arithmetic, schema registry, and error mapping.
- Never: expose a secret/binding/endpoint through WIT; let the guest choose model/URL/idempotency/cost budget; add a native content processor; persist provider response bodies; claim MCP or official component completion; use sub-agents; use `git add -A`.

## 13. Completion criteria

1. `ProviderAiBroker` composes repository, provider policy, admission, `ProviderClient`, exact schema validation, and canonical response mapping.
2. All four existing provider protocols pass deterministic broker tests without real network credentials.
3. Exact user scope, token, concurrency, RPM, cost, idempotency, timeout/error, schema, and locale contracts have rejection tests.
4. Secret, endpoint, prompt, input, schema, provider body, and output sentinels are absent from all formatted errors.
5. `src/plugins/runtime` still contains no provider repository/client/transport/database reference.
6. No artifact, worker, lifecycle, MCP, API/UI, or embedded-component completion claim is made.
7. Format, Clippy, targeted tests, full Rust tests, and CI pass.

## 14. Bounded internal review conclusion

- DDIA: provider records and content jobs remain authoritative; runtime admission is explicitly transient; idempotency and immutable derived-data rules remain intact; no network call enters a database transaction.
- API/interface: the guest still sees one narrow capability; new job identity and cost ceiling remain host-only; exact official schema mapping prevents arbitrary schema semantics from becoming an accidental public contract.
- Security: scope is rechecked at provider load, cost is bounded before I/O, output is typed and canonicalized, and secrets/transport stay behind the broker.
- Scope: this is the missing real provider composition, not an empty abstraction. The official component and MCP schema/tool-plan contract remain explicit next dependencies.
- Open questions: none for this slice. Multi-process admission and MCP tool-schema transport require separate designs before those deployment/feature claims are enabled.
