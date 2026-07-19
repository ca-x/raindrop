# Official AI Component v1 Implementation Plan

> **Execution:** Inline main-Agent execution only. The user explicitly prohibited sub-agent development.

**Goal:** Build and execute the real no-WASI Rust `raindrop.ai-content` component with summary, translation, MCP enrichment, and lifecycle intent behavior.

**Architecture:** A nested Rust guest crate consumes the committed WIT via `wit-bindgen`. Host-side shared tool-plan contracts extend the existing capability and ProviderClient broker without exposing authority to the guest. Integration tests build/componentize the real guest and run it through the hardened Wasmtime host.

**Tech Stack:** Rust 2024/MSRV 1.94, `wit-bindgen = 0.59.0`, `wasm32-unknown-unknown`, `wit-component = 0.253.0`, Wasmtime 46.0.1, Serde canonical JSON.

## Global constraints

- No WASI, filesystem, environment, clock, random, socket, process, provider client, MCP transport, repository, database, or persistence in the guest.
- Generated WIT is the only ABI; no hand-written raw canonical ABI functions.
- At most two provider requests and four MCP calls; lifecycle makes zero capability calls.
- Feed/model/tool content remains untrusted data and never enters system instructions, provenance, logs, or errors.
- Use `apply_patch`, exact `git add`, no sub-agents, no `git add -A`.

---

### Task 1: Add the shared tool-plan contract and broker support

**Files:**

- Create: `src/plugins/tool_plan.rs`
- Modify: `src/plugins/mod.rs`
- Modify: `src/content/ai/schema.rs`
- Test: `src/content/ai/schema.rs`
- Test: `tests/ai_provider_broker.rs`

**Produces:** Canonical dynamic tool-plan schema builder/parser and typed provider output validation.

- [x] Add RED tests for a two-tool canonical `oneOf` schema, max-call drift, duplicate/unknown bindings, non-object arguments, and direct summary/translation regression.
- [x] Implement `ToolPlanSchema` with exact ID/name, canonical builder, strict parser, binding/schema equality, and `validate_output`.
- [x] Extend ProviderAiBroker schema selection to accept the tool-plan family while keeping direct artifact schemas exact.
- [x] Run `cargo test --locked content::ai::schema -- --nocapture` and `cargo test --locked --test ai_provider_broker -- --nocapture`.

### Task 2: Make runtime authorize tool-plan schemas and preserve guest failure codes

**Files:**

- Modify: `src/plugins/runtime/capability.rs`
- Modify: `src/plugins/runtime/error.rs`
- Modify: `src/plugins/runtime/execute.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Test: `tests/plugin_runtime_capabilities.rs`
- Test: `tests/plugin_runtime_sandbox.rs`

**Produces:** Tool-plan schema authorization derived from current descriptors and `PluginFailureCode` access on runtime errors.

- [x] Add RED tests proving schema binding/schema/max-call drift never reaches the AI broker.
- [x] Validate tool-plan schema against the complete `CapabilityToolBinding` map and remaining MCP budget.
- [x] Add an allowlisted fixed `PluginFailureCode` enum; preserve recognized guest message keys while `Debug`/`Display` stay redacted.
- [x] Run capability and sandbox tests.

### Task 3: Scaffold the no-WASI guest and pure contracts

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `plugins/official/raindrop-ai-content/Cargo.toml`
- Create: `plugins/official/raindrop-ai-content/src/lib.rs`
- Create: `plugins/official/raindrop-ai-content/src/config.rs`
- Create: `plugins/official/raindrop-ai-content/src/json.rs`
- Create: `plugins/official/raindrop-ai-content/src/prompt.rs`
- Create: `plugins/official/raindrop-ai-content/src/tool_plan.rs`
- Create: `plugins/official/raindrop-ai-content/src/lifecycle.rs`
- Create: `plugins/official/raindrop-ai-content/src/operation.rs`

**Produces:** Generated component bindings, exact descriptor, strict config/canonical JSON, prompt versions, tool-plan/lifecycle pure logic.

- [x] Add guest unit tests for canonical JSON, config modes, prompt isolation, schema golden output, plan bounds, lifecycle dedupe/order/idempotency.
- [x] Implement the minimal modules until native guest tests pass.
- [x] Generate/export the committed WIT world only on `wasm32` and compile the exact descriptor.
- [x] Install/check `wasm32-unknown-unknown` and run guest native tests plus release target check.

### Task 4: Implement direct summary and translation execution

**Files:**

- Modify: guest `src/operation.rs`, `src/prompt.rs`, and `src/lib.rs`
- Test: guest unit tests
- Test: `tests/official_ai_component.rs`

**Produces:** Ordinal-1 direct generation and exact artifact candidates.

- [x] Add real-component RED tests capturing the AI request for summary and translation.
- [x] Implement canonical untrusted input, conservative input token ceiling, final output budgeting, finish-reason checks, exact schemas, and safe provenance.
- [x] Return disabled/config/provider/output failures with fixed message keys.
- [x] Run direct component tests.

### Task 5: Implement MCP plan, calls, and failure policy

**Files:**

- Modify: guest `src/tool_plan.rs`, `src/operation.rs`, and `src/lib.rs`
- Test: guest unit tests
- Test: `tests/official_ai_component.rs`

**Produces:** Two-request MCP enrichment with applied, degraded, and fail-closed outcomes.

- [x] Add RED tests for exact dynamic schema, valid two-call flow, unknown/duplicate binding, MCP failure open, MCP failure closed, aggregate input limit, and no third provider call.
- [x] Implement ordinal-1 plan, sequential 10-second calls, canonical result projection, ordinal-2 final generation, and complete result discard on fail-open.
- [x] Map every host MCP error to a fixed guest failure key.
- [x] Run MCP component tests.

### Task 6: Build and execute the real component deterministically

**Files:**

- Create: `tests/support/official_ai_component.rs`
- Create: `tests/official_ai_component.rs`
- Modify: `tests/support/mod.rs`
- Modify: `.github/workflows/ci.yml`

**Produces:** Locked real guest build, deterministic component bytes, no-WASI inspection, signed host compilation, and Wasmtime behavior tests.

- [x] Add a build helper that removes ambient `RUSTUP_TOOLCHAIN`, invokes the pinned repo toolchain/target with `--locked --release`, and componentizes with validation.
- [x] Encode identical core bytes twice and require byte equality.
- [x] Inspect component text/imports for only committed host interfaces and no `wasi:` package.
- [x] Add CI target installation and a focused official-component test step before the full suite.
- [x] Run `cargo test --locked --test official_ai_component -- --nocapture --test-threads=1`.

### Task 7: Verify, document, review once, commit, and push

**Files:**

- Modify: `tasks/plan.md`
- Modify: `tasks/todo.md`
- Modify: this plan's checkboxes

- [x] Run all commands from the design specification, including guest native/wasm checks, targeted host tests, Clippy, and the full Rust suite.
- [x] Run guest/host source-confinement searches and `git diff --check`.
- [x] Perform one bounded contract/security review and fix only confirmed findings.
- [x] Stage exact files, inspect the staged diff, scan for real secret patterns.
- [ ] Commit with `feat: build official AI content component` and push `main`.
- [ ] Monitor the pushed CI run before starting content-worker composition.

## Self-review

- Spec coverage: descriptor, direct operations, dynamic MCP schema, failure policies, lifecycle intents, failure-code preservation, deterministic build, CI target, and no false worker/embedding claim each map to a task.
- Type consistency: host `ToolPlanSchema` and guest schema builder use the same ID/layout; runtime failure enum consumes fixed guest message keys; real component tests exercise generated WIT types.
- Scope: no transport, database, worker, dispatcher, artifact commit, API/UI, production signing, or marketplace work is included.
