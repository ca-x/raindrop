# AI Provider Broker Composition v1 Implementation Plan

> **Execution:** Inline main-Agent execution is required. The user explicitly prohibited sub-agent development. Track every step with the checkboxes below and commit the completed vertical slice once its full verification gate passes.

**Goal:** Connect the Wasmtime AI capability to the real encrypted provider core with bounded admission, stable idempotency, exact official schema validation, and redacted error semantics.

**Architecture:** `src/plugins/runtime` remains a transport-free sandbox boundary. A new `src/content/ai` module composes `ProviderRepository`, process-local provider admission, and generic `ProviderClient<T>`, validates only the two committed official artifact schemas, and returns a canonical `AiBrokerResponse`. Content jobs/artifacts remain database-authoritative and are not executed or written in this slice.

**Tech Stack:** Rust 1.94+ edition 2024, Tokio synchronization/time, SeaORM SQLite test fixture, existing BLAKE3/Serde JSON/provider adapters, existing Wasmtime capability traits.

## Global constraints

- Do not add a native summarize/translate processor or call provider code from `src/plugins/runtime`.
- Do not add database tables, provider API/UI, content worker, artifact commit, lifecycle dispatch, MCP transport, or official component completion claims.
- Do not expose provider credentials, endpoints, prompts, schemas, inputs, outputs, or raw upstream errors through formatting.
- Derive provider idempotency from host-issued job ID plus request ordinal; do not accept it from WIT.
- Use non-blocking bounded admission; do not create an unbounded wait queue.
- Use `apply_patch`, targeted `git add`, and no sub-agents or `git add -A`.

---

### Task 1: Strengthen the host-only broker context

**Files:**

- Modify: `src/plugins/runtime/capability.rs`
- Modify: `src/plugins/runtime/execute.rs`
- Modify: `tests/plugin_runtime_capabilities.rs`
- Modify: `tests/plugin_runtime_sandbox.rs`

**Interfaces:**

- Produces `BrokerInvocationContext::job_id` and `AiBrokerRequest::max_cost_micros` for the production broker.
- Preserves the WIT ABI; neither field is guest-controlled.

- [x] **Step 1: Add failing capability tests**

Add assertions proving:

```rust
let mut mismatched = operation_request(...);
mismatched.job_id = "other-job".to_owned();
assert_eq!(
    runtime.execute(&compiled, session, mismatched).await.unwrap_err().kind(),
    PluginRuntimeErrorKind::InvalidInvocation,
);
```

The mock AI broker must observe:

```rust
assert_eq!(context.job_id, "job-1");
assert_eq!(request.max_cost_micros, 250_000);
```

Also add a translation artifact case whose artifact locale matches the request but whose payload `targetLocale` differs; expect `InvalidInvocation`.

- [x] **Step 2: Run RED tests**

```bash
cargo test --locked --test plugin_runtime_capabilities -- --nocapture
cargo test --locked --test plugin_runtime_sandbox -- --nocapture
```

Expected: compile/assertion failure because the host-only fields and locale invariant do not yet exist.

- [x] **Step 3: Implement the host-only fields and validation**

Extend the internal structures:

```rust
pub struct BrokerInvocationContext {
    pub invocation_id: String,
    pub job_id: String,
    pub user_subject: String,
    pub call_chain_id: String,
    pub operation: types::Operation,
    pub trigger: types::Trigger,
    pub remaining_depth: u32,
}

pub struct AiBrokerRequest {
    // existing fields...
    pub max_cost_micros: u64,
}
```

Exclude `job_id` from custom `Debug`. Validate it as bounded visible ASCII, require `request.job_id == invocation.job_id`, and populate `max_cost_micros` from the session remaining-cost budget.

In `validate_artifact`, parse translation once and require:

```rust
translation.target_locale() == request.target_locale.as_deref().unwrap_or_default()
```

- [x] **Step 4: Run GREEN tests**

```bash
cargo test --locked --test plugin_runtime_capabilities -- --nocapture
cargo test --locked --test plugin_runtime_sandbox -- --nocapture
```

Expected: both pass.

---

### Task 2: Add checked cost and provider admission primitives

**Files:**

- Create: `src/content/ai/mod.rs`
- Create: `src/content/ai/cost.rs`
- Create: `src/content/ai/admission.rs`
- Modify: `src/content/mod.rs`
- Test: `tests/ai_provider_broker.rs`

**Interfaces:**

- Produces `ProviderAdmissionController::acquire(&ProviderMetadata)` and `estimate_cost` helpers consumed by Task 4.

- [x] **Step 1: Write failing primitive tests**

Cover exact arithmetic and admission behavior:

```rust
assert_eq!(estimate_cost(Some(1_000), Some(2_000), 1_500, 500), Some(3));
assert_eq!(estimate_cost(None, Some(2_000), 1_500, 500), None);
```

Create provider metadata with `max_concurrency = 1`, hold one permit, and require the second non-blocking acquire to return retryable `RateLimited`. With `requests_per_minute = Some(1)`, release the permit and require a second reservation inside 60 seconds to return `RateLimited` with a retry time.

- [x] **Step 2: Run RED test**

```bash
cargo test --locked --test ai_provider_broker admission -- --nocapture --test-threads=1
```

Expected: compile failure because the AI module does not exist.

- [x] **Step 3: Implement checked cost arithmetic**

Use `u128` multiplication and ceiling division:

```rust
fn token_cost(tokens: u64, rate: u64) -> Option<u64> {
    let numerator = u128::from(tokens).checked_mul(u128::from(rate))?;
    u64::try_from(numerator.div_ceil(1_000_000)).ok()
}
```

Return `None` unless both input and output prices are present. Treat overflow as broker invalid/quota failure rather than wrapping.

- [x] **Step 4: Implement process-local admission**

Use a controller-owned Tokio mutex map keyed by `(provider_id, revision)` and an admission state containing:

```rust
struct ProviderAdmissionState {
    concurrency: Arc<Semaphore>,
    request_window: Mutex<VecDeque<Instant>>,
    requests_per_minute: Option<u32>,
}
```

Acquire with `try_acquire_owned`, purge timestamps whose age is at least 60 seconds, and record the new request reservation only after both gates pass. Return an owned guard that keeps the concurrency permit alive through the provider call.

- [x] **Step 5: Run GREEN primitive tests**

```bash
cargo test --locked --test ai_provider_broker admission -- --nocapture --test-threads=1
```

Expected: pass.

---

### Task 3: Freeze the official schema registry

**Files:**

- Create: `src/content/ai/schema.rs`
- Modify: `src/content/ai/mod.rs`
- Test: `tests/ai_provider_broker.rs`

**Interfaces:**

- Produces `OfficialSchema::validate_request` and `OfficialSchema::validate_output` for Task 4.

- [x] **Step 1: Add failing schema tests**

Test exact pairs and hostile variants:

```rust
assert!(OfficialSchema::for_operation(Operation::Summarize)
    .validate_request(SUMMARY_SCHEMA_ID, canonical_summary_schema()).is_ok());
assert!(OfficialSchema::for_operation(Operation::Summarize)
    .validate_request(TRANSLATION_SCHEMA_ID, canonical_translation_schema()).is_err());
```

Reject an alternative document with the correct `$id`, a non-canonical request string, duplicate keys, malformed summary output, raw HTML, and translation output whose target locale differs from the request input.

- [x] **Step 2: Run RED schema tests**

```bash
cargo test --locked --test ai_provider_broker schema -- --nocapture --test-threads=1
```

Expected: compile failure because the registry does not exist.

- [x] **Step 3: Implement exact schema mapping**

Embed the committed files with `include_str!`, parse to `serde_json::Value`, and canonicalize through a duplicate-rejecting parser/equality check. Map:

```rust
Summarize => ("raindrop://schemas/artifacts/ai-summary/v1", "raindrop_ai_summary_v1")
Translate => ("raindrop://schemas/artifacts/ai-translation/v1", "raindrop_ai_translation_v1")
```

Validate output through existing typed artifact parsers and return their canonical JSON. For translation, extract the canonical input `targetLocale` and require exact normalized equality with the typed output.

- [x] **Step 4: Run GREEN schema tests**

```bash
cargo test --locked --test ai_provider_broker schema -- --nocapture --test-threads=1
```

Expected: pass.

---

### Task 4: Compose ProviderRepository and ProviderClient

**Files:**

- Create: `src/content/ai/broker.rs`
- Modify: `src/content/ai/mod.rs`
- Modify: `src/plugins/runtime/capability.rs`
- Test: `tests/ai_provider_broker.rs`

**Interfaces:**

- Produces public `ProviderAiBroker<T>` implementing `AiCapabilityBroker`.

- [x] **Step 1: Add failing four-adapter composition tests**

For Anthropic Messages, OpenAI Responses, OpenAI Chat Completions, and Google Gemini:

1. create a real encrypted instance provider in SQLite;
2. construct `ProviderAiBroker<RecordingTransport>`;
3. call `generate_structured` with the exact summary schema;
4. assert one transport call and the canonical full summary artifact response;
5. assert stable idempotency header/body behavior for the same job/ordinal.

Also add wrong-user, disabled-provider, token/cost-limit, invalid schema, provider malformed output, timeout, 429, upstream, RPM, and concurrency cases.

- [x] **Step 2: Run RED composition tests**

```bash
cargo test --locked --test ai_provider_broker broker -- --nocapture --test-threads=1
```

Expected: compile failure because the broker does not exist.

- [x] **Step 3: Implement broker execution**

The implementation order is fixed:

```rust
let schema = OfficialSchema::validate_request(...)?;
let binding = repository.load_enabled_binding(...).await.map_err(map_repository)?;
validate_provider_policy(binding.metadata().policy(), &request)?;
let maximum_cost = maximum_request_cost(...)?;
let _admission = admission.acquire(binding.metadata()).await?;
let idempotency_key = provider_call_idempotency_key(&context.job_id, request.provider_request_ordinal);
let response = client.generate(&binding, &provider_request).await.map_err(map_provider_call)?;
let output_json = schema.validate_output(response.output, &input)?;
```

Convert finish reason and usage without exposing provider payloads. Return actual estimated cost when usage is complete, otherwise the admitted conservative estimate.

- [x] **Step 4: Expose safe broker error accessors**

Add read-only `retryable()` and `retry_at_unix_ms()` accessors to `AiBrokerError` for deterministic tests. Do not add payload or source accessors.

- [x] **Step 5: Run GREEN composition tests**

```bash
cargo test --locked --test ai_provider_broker -- --nocapture --test-threads=1
```

Expected: all broker tests pass.

---

### Task 5: Verify, document progress, commit, and push

**Files:**

- Modify: `tasks/plan.md`
- Modify: `tasks/todo.md` only if the exact broker sub-capability is represented; keep the overall official component/API/UI/MCP items unchecked.
- Modify: `docs/superpowers/plans/2026-07-19-ai-provider-broker-composition-v1.md` checkboxes.

- [x] **Step 1: Run formatting and targeted gates**

```bash
cargo fmt --check
cargo test --locked --test ai_provider_broker -- --nocapture --test-threads=1
cargo test --locked --test plugin_runtime_capabilities -- --nocapture
cargo test --locked --test plugin_runtime_sandbox -- --nocapture
```

- [x] **Step 2: Run static and full gates**

```bash
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

- [x] **Step 3: Perform one bounded internal review**

Check:

- runtime source confinement;
- no network call inside a database transaction;
- no secret/prompt/input/schema/output in formatting;
- exact schema and locale invariants;
- process-local admission limitation documented truthfully;
- official component/MCP/API/UI/job completion still unclaimed.

- [x] **Step 4: Record progress**

Update `tasks/plan.md` so the next dependency is the official `raindrop.ai-content` component and its MCP tool-schema decision, followed by the content worker.

- [x] **Step 5: Commit and push**

```bash
git add \
  docs/superpowers/specs/2026-07-19-ai-provider-broker-composition-v1-design.md \
  docs/superpowers/plans/2026-07-19-ai-provider-broker-composition-v1.md \
  src/content/ai \
  src/content/mod.rs \
  src/plugins/runtime/capability.rs \
  src/plugins/runtime/execute.rs \
  tests/ai_provider_broker.rs \
  tests/plugin_runtime_capabilities.rs \
  tests/plugin_runtime_sandbox.rs \
  tasks/plan.md \
  tasks/todo.md
git commit -m "feat: compose AI provider broker"
git push origin main
```

Expected: push succeeds and CI starts for the new main commit.

## Self-review

- Spec coverage: provider scope, all four adapters, concurrency/RPM, token/cost, idempotency, exact schemas, typed output, locale, error mapping, redaction, and runtime confinement each map to a task.
- Placeholder scan: no implementation `TODO`, `TBD`, or unspecified error handling remains.
- Type consistency: `job_id` belongs to `BrokerInvocationContext`; `max_cost_micros` belongs to `AiBrokerRequest`; `ProviderAiBroker<T>` consumes `ProviderRepository`, `ProviderClient<T>`, and `ProviderAdmissionController` exactly once.
- Scope: official component, content worker, lifecycle, MCP, provider UI/API, and artifact UI remain explicit next work rather than being marked complete by this broker slice.
