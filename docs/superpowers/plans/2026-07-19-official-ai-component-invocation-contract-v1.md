# Official AI Component Invocation Contract v1 Implementation Plan

> **Execution:** Inline main-Agent execution only. The user prohibited sub-agent development.

**Goal:** Correct the unshipped WIT v1 so the future official component can receive exact MCP tool schemas and per-user lifecycle config without receiving authority or secrets.

**Architecture:** The WIT tool descriptor becomes a complete, non-secret planning snapshot owned and validated by `CapabilitySession`. Lifecycle invocation gains a config-bearing wrapper and remains capability-suspended. Generated bindings remain the sole ABI source, and existing dummy components regenerate from the committed WIT.

**Tech Stack:** Rust 2024, Wasmtime Component Model generated bindings, WIT 1.0.0, BLAKE3, canonical Serde JSON, existing plugin validators.

## Global constraints

- No provider/MCP/database/transport type enters `src/plugins/runtime`.
- No lifecycle callback may activate AI/MCP imports.
- No executable official component, MCP transport, worker, dispatcher, API/UI, or artifact completion claim in this slice.
- Correct the existing WIT 1.0.0 directly because it has not shipped; do not add a fake compatibility layer.
- Use `apply_patch`, targeted `git add`, no sub-agents, and no `git add -A`.

---

### Task 1: Correct WIT and generated binding expectations

**Files:**

- Modify: `contracts/wit/raindrop-content-plugin-v1/types.wit`
- Modify: `contracts/wit/raindrop-content-plugin-v1/content-plugin.wit`
- Modify: `tests/plugin_runtime_bindings.rs`
- Modify: `tests/plugin_contract_assets.rs`

- [x] Add compile-time generated-type tests that construct the enriched `types::ToolBinding` and `types::LifecycleRequest`.
- [x] Run the binding/asset tests and confirm RED compilation against the old WIT.
- [x] Add connection/tool/description/schema/digest fields and `lifecycle-request`; change `on-event` to accept it.
- [x] Run the binding/asset tests and confirm GREEN.

### Task 2: Make the capability session own complete tool descriptors

**Files:**

- Modify: `src/plugins/runtime/capability.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Modify: `tests/plugin_runtime_capabilities.rs`

- [x] Add tests for valid descriptors, duplicate IDs, duplicate connection/tool pairs, invalid names/UUIDs, label/description/schema bounds, non-canonical schema, digest mismatch, and redacted `Debug`.
- [x] Implement `CapabilityToolBindingInput` and `CapabilityToolBinding` with domain-separated schema hashing.
- [x] Replace `CapabilitySessionConfig::tool_binding_ids` with `tool_bindings` and map them by binding ID.
- [x] Require the WIT operation request tool descriptors to equal the session descriptors exactly.
- [x] Run capability tests and confirm GREEN.

### Task 3: Validate corrected operation request size and descriptor drift

**Files:**

- Modify: `src/plugins/runtime/execute.rs`
- Modify: `tests/plugin_runtime_sandbox.rs`

- [x] Add request-drift tests for connection ID, tool name, description, schema, and digest.
- [x] Include every new tool field in operation request size accounting.
- [x] Freeze complete descriptor and lifecycle wrapper accounting at the exact 1-MiB N/N+1 boundary.
- [x] Keep the existing 16-binding and 1-MiB request ceilings.
- [x] Run sandbox tests and confirm GREEN.

### Task 4: Add config-bearing suspended lifecycle invocation

**Files:**

- Modify: `src/plugins/runtime/execute.rs`
- Verify: `tests/support/plugin_component.rs` (the fixture is generated directly from committed WIT, so no manual source edit is required)
- Modify: `tests/plugin_runtime_sandbox.rs`

- [x] Change runtime `on_event` to accept `types::LifecycleRequest`.
- [x] Validate compiled plugin identity, invocation ID, canonical `AiContentConfig`, config hash, event contract, event subject, and descriptor subscription.
- [x] Keep the capability session suspended; remove lifecycle activation.
- [x] Update dummy component generation and lifecycle test fixtures for the new ABI.
- [x] Combine the suspended-session import denial tests with a source-confinement assertion that `on_event` never activates the session.
- [x] Run sandbox/capability tests and confirm GREEN.

### Task 5: Verify, document, commit, and push

**Files:**

- Modify: parent specifications where their WIT prose is now stale.
- Modify: `tasks/plan.md`.
- Modify: this plan's checkboxes.

- [x] Run `cargo fmt --check`.
- [x] Run binding, contract, capability, and sandbox targeted tests.
- [x] Run `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --locked --all-features` locally and from an isolated detached worktree.
- [x] Run `git diff --check` and the runtime source-confinement search.
- [x] Perform one bounded contract/security review; add the missing exact aggregate-size boundary tests and stop the review loop.
- [x] Commit with `feat: correct official plugin invocation contract` and push `main`.

## Self-review

- Spec coverage: MCP planning metadata, config-bearing lifecycle, descriptor equality, schema digest, limits, suspended lifecycle capabilities, generated bindings, and no false feature claim each map to a task.
- Type consistency: WIT `tool-binding` maps to `CapabilityToolBinding`; WIT `lifecycle-request` maps directly to the runtime `on_event` input.
- Scope: no executable component or MCP/dispatcher implementation is smuggled into this contract repair.
