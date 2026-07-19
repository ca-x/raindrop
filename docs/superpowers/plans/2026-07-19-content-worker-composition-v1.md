# Content Worker Composition v1 Implementation Plan

> **Execution:** Inline main-Agent execution only. The user explicitly prohibited sub-agent development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compose durable content claims, the official AI Wasm component, provider broker, heartbeat, and atomic artifact terminalization into one recoverable worker runtime.

**Architecture:** Extend existing claims and capability execution with the minimum accounting/snapshot data, add a focused official processor, then run it under eight database-backed worker lanes. All Wasm/provider work stays outside transactions; existing fenced terminal methods remain the only persistence path.

**Tech Stack:** Rust 2024/MSRV 1.94, Tokio, SeaORM, Wasmtime component model, Serde canonical JSON, BLAKE3, existing provider/plugin/job domains.

## Global Constraints

- Main Agent only; no sub-agents and one bounded internal review.
- No database transaction across Wasm, provider, or future MCP calls.
- Database time and fencing decide commit authority.
- No schema migration or new dependency in this slice.
- Direct summary/translation only; MCP-enabled config fails closed until MCP transport exists.
- Exact `apply_patch` edits, exact staging, no `git add -A`.

---

### Task 1: Complete claim and execution-entry contracts

**Files:**

- Modify: `src/content/jobs/model.rs`
- Modify: `src/content/jobs/repository.rs`
- Modify: `src/content/jobs/sql.rs`
- Test: `tests/content_job_claims.rs`
- Test: `tests/content_worker_processor.rs`

**Interfaces:**

- Produces: added `ContentJobClaim::{operation,trigger,idempotency_key,call_chain_id,remaining_depth}` accessors.
- Produces: `ContentExecutionEntry` and `ContentRepository::load_execution_entry`.
- Produces: `LeaseDeadline::remaining_attempt()` from database time.

- [x] Add claim tests asserting the complete persisted execution context survives claim and recovery.
- [x] Add entry snapshot tests for active visibility, exact content hash, decoded sanitized content, rendered text, and stale/corrupt rejection.
- [x] Implement the claim fields directly from the locked job row; do not accept caller-provided execution metadata.
- [x] Implement a single bounded visible-entry query plus `EntryContentDetail::decode` and rendered-text extraction.
- [x] Compute remaining attempt duration inside heartbeat from the same `database_now` sample used for validation.
- [x] Run `cargo test --locked --test content_job_claims -- --nocapture` and the focused snapshot tests.

### Task 2: Freeze invocation hashes and provider snapshot authorization

**Files:**

- Create: `src/content/worker/input.rs`
- Create: `src/content/worker/mod.rs`
- Modify: `src/content/mod.rs`
- Modify: `src/plugins/config.rs`
- Modify: `src/plugins/runtime/capability.rs`
- Modify: `src/content/ai/broker.rs`
- Test: `tests/ai_provider_broker.rs`
- Test: `tests/content_worker_processor.rs`

**Interfaces:**

- Produces: `ContentInvocationInput::new`, `canonical_json`, `hash`, `to_wit_entry`.
- Produces: `disabled_mcp_provenance_hash()`.
- Produces: operation-specific plugin config accessors for enabled/provider/max-output/MCP mode.
- Extends: `BrokerInvocationContext` with expected provider kind/model/revision.

- [x] Add golden tests for canonical input JSON/hash and the disabled-MCP provenance hash.
- [x] Add provider broker tests proving revision, kind, and model drift never reaches transport.
- [x] Implement the exact schema-v1 canonical input builder with 512 KiB text and WIT field bounds.
- [x] Add read-only operation config accessors without exposing mutable config internals.
- [x] Validate provider snapshot fields in `CapabilitySession` and `ProviderAiBroker` after binding load.
- [x] Run the input and provider broker tests.

### Task 3: Preserve detailed plugin usage and retry hints

**Files:**

- Modify: `src/plugins/runtime/capability.rs`
- Modify: `src/plugins/runtime/error.rs`
- Modify: `src/plugins/runtime/execute.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Test: `tests/plugin_runtime_capabilities.rs`
- Test: `tests/plugin_runtime_sandbox.rs`
- Test: `tests/official_ai_component.rs`

**Interfaces:**

- Produces: `CapabilityUsage`, `CapabilityFailureHint`.
- Produces: `PluginExecutionSuccess`, `PluginExecutionFailure`.
- Produces: `PluginRuntime::execute_detailed`; existing `execute` remains compatible.

- [x] Add tests for attempted call counts, summed tokens/cost, completeness flags, final model label, retry-at preservation, and timeout outcome-unknown.
- [x] Add tests proving fail-open success ignores an intermediate MCP failure hint and arbitrary guest messages remain discarded.
- [x] Record usage only from host broker responses; never trust guest-provided metrics.
- [x] Capture usage from the store on guest success, guest error, output validation error, and trap.
- [x] Implement `execute` as a wrapper over `execute_detailed`.
- [x] Run capability, sandbox, and real-component tests.

### Task 4: Implement the official content processor

**Files:**

- Create: `src/content/worker/error.rs`
- Create: `src/content/worker/processor.rs`
- Modify: `src/content/worker/mod.rs`
- Test: `tests/content_worker_processor.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**

- Produces: `ContentProcessor` async trait.
- Produces: `ContentProcessSuccess`, `ContentProcessFailure`.
- Produces: `OfficialAiProcessor::new` and `process`.

- [x] Write real-component tests for direct summary and translation through `ProviderAiBroker` and a recording transport.
- [x] Test entry/config/provider/plugin/prompt/schema/MCP drift and stable failure mapping.
- [x] Implement preflight reads and exact snapshot comparisons before creating the capability session.
- [x] Build the WIT request only through `ContentInvocationInput`, fixed operation contracts, and claim data.
- [x] Convert detailed host usage into bounded `AttemptUsage`; use the final host model label as artifact provider label.
- [x] Map only fixed host/plugin codes into `AttemptFailure`, including bounded Retry-After and outcome-unknown.
- [x] Run `cargo test --locked --test content_worker_processor -- --nocapture --test-threads=1`.

### Task 5: Implement heartbeat and eight-lane runtime

**Files:**

- Create: `src/content/worker/runtime.rs`
- Modify: `src/content/worker/error.rs`
- Modify: `src/content/worker/mod.rs`
- Create: `tests/content_worker_runtime.rs`

**Interfaces:**

- Produces: `ContentWorker::new`, `run_claim`.
- Produces: `ContentRuntime::new`, `ContentRuntime::run`, `ContentRuntimeHandle::{notify,shutdown}`.

- [x] Add fake-processor tests for success terminalization, retry scheduling, heartbeat extension, lease-loss cancellation, terminal/heartbeat ordering, wake-up, eight-lane ceiling, and 30-second bounded shutdown.
- [x] Perform an immediate heartbeat before processor execution and pass its DB-derived remaining duration.
- [x] Race processing against a 10-second heartbeat loop; on heartbeat failure drop processing and perform no completion.
- [x] Stop and join heartbeat before calling `complete_success` or `complete_failure`.
- [x] Run exactly eight supervised lane loops with one-second polling and notification wake-up.
- [x] Run `cargo test --locked --test content_worker_runtime -- --nocapture --test-threads=1`.

### Task 6: Verify, document, review once, commit, and push

**Files:**

- Modify: `tasks/plan.md`
- Modify: `tasks/todo.md`
- Modify: this plan's checkboxes

- [x] Run all commands from the design specification.
- [x] Verify no transaction spans processor calls and no secret/untrusted text enters errors or metadata.
- [x] Perform one bounded DDIA/contract/security review and fix only confirmed findings.
- [x] Run source confinement, real secret pattern scan, and `git diff --check`.
- [x] Stage exact files and inspect the staged diff.
- [ ] Commit and push `main`, then monitor the corresponding GitHub Actions run.

## Self-review

- Spec coverage: claim context, snapshot hashing, provider drift, usage, real component processing, heartbeat, runtime, terminal commit, and explicit exclusions each map to one task.
- Type consistency: the processor consumes only repository claims and DB-derived duration; runtime consumes only `ContentProcessor`; terminal methods continue to consume existing `ArtifactCandidate`, `AttemptUsage`, and `AttemptFailure`.
- DDIA: database is the record system, external calls are transaction-free, delivery is at least once, and fencing plus immutable identity provide convergence.
- Scope: no API, lifecycle delivery, MCP transport, Reader UI, signed embedding, or `main` wiring is marked complete.
