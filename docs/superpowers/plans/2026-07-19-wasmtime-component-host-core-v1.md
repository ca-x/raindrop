# Wasmtime Component Host Core v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `executing-plans` inline. Sub-agent execution is forbidden by user instruction. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add the real hardened Wasmtime Component Model host and invocation capability session required by the official AI plugin.

**Architecture:** Generated bindings consume the committed WIT. A single hardened engine compiles only verified bundled binary components, while every invocation receives a fresh limited Store, exact linker, descriptor gate, capability session, fuel, and paused-host-time epoch budget. Broker traits remain capability abstractions; no provider or MCP transport is called in this slice.

**Tech Stack:** Rust 2024 / Rust 1.94, Wasmtime 46.0.1, Tokio, async-trait, Serde JSON, ring SHA-256, wit-component 0.253.0 test fixtures.

## Global Constraints

- No sub-agents, WASI, arbitrary upload, direct ProviderClient/MCP transport, database access, artifact commit, or Feed transaction execution.
- Component binary <= 16 MiB; memory <= 64 MiB; execute fuel 50M; lifecycle fuel 5M; cumulative guest CPU <= 2s.
- WIT request <= 1 MiB; artifact/event output <= 512 KiB; AI input <= 512 KiB/schema <= 64 KiB; MCP args <= 64 KiB/result <= 256 KiB.
- Use public seam TDD and push immediately after every commit.
- Do not touch `.superpowers/research/`, root `node_modules/`, frontend, routes, or existing provider/content persistence semantics.

---

### Task 1: Commit binding specification and plan

**Files:**
- Create: `docs/superpowers/specs/2026-07-19-wasmtime-component-host-core-v1-design.md`
- Create: `docs/superpowers/plans/2026-07-19-wasmtime-component-host-core-v1.md`

**Interfaces:**
- Consumes: official AI plugin parent design and completed contract/registry core.
- Produces: exact dependency/features, limits, broker boundary, tests, and completion gates.

- [ ] Run placeholder and `git diff --check` scans.
- [ ] Reconcile every parent resource limit and sandbox requirement with Tasks 2-5.
- [ ] Commit exact documents as `docs: design wasmtime component host core` and push.

### Task 2: Add Wasmtime dependency and generated bindings

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/plugins/runtime/mod.rs`
- Create: `src/plugins/runtime/bindings.rs`
- Modify: `src/plugins/mod.rs`
- Create: `tests/plugin_runtime_bindings.rs`

**Interfaces:**
- Consumes: `contracts/wit/raindrop-content-plugin-v1`.
- Produces: generated async typed bindings and compile-time names used by later tasks.

- [ ] Write a red test importing the generated package/world and checking host/guest type identity.
- [ ] Run `cargo test --test plugin_runtime_bindings`; expect missing runtime bindings.
- [ ] Add exact Wasmtime 46.0.1 with only `async,call-hook,component-model,component-model-async,cranelift,runtime,std`.
- [ ] Add `wit-component 0.253.0` with `dummy-module` and `wasmprinter 0.253.0` as dev dependencies.
- [ ] Generate bindings with async/trappable imports and async exports.
- [ ] Run the binding test, Clippy, and feature-tree assertion.
- [ ] Commit as `feat: generate plugin runtime bindings` and push.

### Task 3: Implement hardened engine and verified component compilation

**Files:**
- Create: `src/plugins/runtime/error.rs`
- Create: `src/plugins/runtime/engine.rs`
- Create: `src/plugins/runtime/component.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Create: `tests/plugin_runtime_component.rs`
- Create: `tests/support/plugin_component.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**
- Consumes: `BundledOfficialPlugin`, component bytes, generated WIT world.
- Produces: `PluginRuntime::new`, `CompiledPlugin::compile`, stable `PluginRuntimeErrorKind`, deterministic component fixture helpers.

- [ ] Red-test exact signed component compile, digest mismatch, empty/oversized input, malformed component, and WAT rejection.
- [ ] Configure component async/fuel/epoch/cranelift engine and 10 ms epoch task.
- [ ] Recompute SHA-256 before `Component::from_binary`; never use unsafe deserialize or WAT parsing.
- [ ] Add redacted errors and immutable compiled descriptor fields.
- [ ] Run targeted tests and commit as `feat: compile verified plugin components`; push.

### Task 4: Implement invocation capability session and brokers

**Files:**
- Create: `src/plugins/runtime/capability.rs`
- Create: `src/plugins/runtime/host.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Create: `tests/plugin_runtime_capabilities.rs`

**Interfaces:**
- Consumes: invocation context and generated host-ai/host-mcp request/response types.
- Produces: `AiCapabilityBroker`, `McpCapabilityBroker`, deny brokers, broker DTOs/errors, `CapabilitySession`, generated Host implementations.

- [ ] Red-test exact AI binding/operation/ordinal/call/token/JSON limits, broker timeout/output validation, and redaction.
- [ ] Implement AI DTO validation and mock-broker success/error mapping.
- [ ] Red-test exact MCP tool/depth/call/timeout/args/result limits and default denial.
- [ ] Implement MCP validation and deny-by-default broker.
- [ ] Implement generated host traits as thin delegates with no transport/repository access.
- [ ] Run targeted tests and commit as `feat: broker plugin host capabilities`; push.

### Task 5: Instantiate and execute with descriptor, fuel, memory, and guest CPU gates

**Files:**
- Create: `src/plugins/runtime/execute.rs`
- Modify: `src/plugins/runtime/engine.rs`
- Modify: `src/plugins/runtime/mod.rs`
- Create: `tests/plugin_runtime_sandbox.rs`

**Interfaces:**
- Consumes: `CompiledPlugin`, `CapabilitySession`, generated guest calls.
- Produces: `PluginRuntime::{execute,on_event}` returning validated guest data without persistence.

- [ ] Generate deterministic dummy/hostile components from WIT in tests.
- [ ] Red-test unexpected import link denial, descriptor trap/mismatch, generic trap, memory >64 MiB, execute/lifecycle fuel distinction, and 2s pure guest timeout.
- [ ] Add fresh Store limits, call-hook guest-time accounting, epoch deadlines, fuel, exact linker, and descriptor-before-call flow.
- [ ] Validate request/output bounds and stable error mapping.
- [ ] Add source-confinement test forbidding WASI/direct ProviderClient/database/transport references.
- [ ] Run all runtime tests and commit as `feat: execute sandboxed plugin components`; push.

### Task 6: Record status and final verification

**Files:**
- Modify: `tasks/plan.md`
- Modify: `tasks/todo.md`
- Create: `.superpowers/sdd/wasmtime-component-host-core-v1-report.md`
- Update spec/plan only for implementation-discovered binding decisions.

**Interfaces:**
- Consumes: verified Tasks 2-5.
- Produces: accurate status leaving ProviderClient broker, official component, lifecycle, MCP, and UI pending.

- [ ] Mark the detailed plan complete and update global task layering.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run targeted runtime tests.
- [ ] Run `cargo test --all-targets`.
- [ ] Run diff/status/secret scans.
- [ ] Commit as `docs: record wasmtime component host core` and push.
- [ ] Follow the single resulting CI run through Rust service databases, Windows, ASTRYX, release E2E, and non-root container; fix only explicit failures.
