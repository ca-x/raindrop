# AI Provider Adapters v1 Implementation Plan

> **For agentic workers:** Execute inline in the main Agent only. Do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement four real provider protocol adapters behind one bounded, redacted canonical structured-generation contract.

**Architecture:** `src/content/provider` owns every provider DTO and maps canonical requests to provider JSON and provider JSON back to canonical responses. It does no storage or network I/O; later provider core code supplies authorized credentials/endpoints and executes the encoded request through a separate SSRF-safe transport.

**Tech Stack:** Rust 1.94.0 edition 2024, `http`, `secrecy`, `serde`, `serde_json`, `thiserror`, `url`, existing Cargo lockfile.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-ai-provider-adapters-v1-design.md` exactly.
- Main Agent only; do not use subagents.
- Do not add a Rust dependency or modify Web code.
- Do not modify `.superpowers/research/` or root `node_modules/`.
- No real provider request, credential, endpoint, database schema, HTTP route, Wasm host, MCP code, or UI in this slice.
- Provider-specific field names stay inside `src/content/provider` and `tests/fixtures/ai-provider/v1`.
- Every commit is independently formatted, tested, pushed, and monitored only for concrete CI failure.

---

### Task 1: Canonical bounded and redacted protocol

**Files:**
- Create: `src/content/provider/mod.rs`
- Create: `src/content/provider/types.rs`
- Create: `src/content/provider/validation.rs`
- Modify: `src/content/mod.rs`
- Create: `tests/ai_provider_adapters.rs`

**Interfaces:**
- Produces `ProviderKind`, `OutputSchema`, `StructuredGenerationRequest`, `ProviderHeader`, `EncodedProviderRequest`, `StructuredGenerationResponse`, `TokenUsage`, `FinishReason`, `ProviderAdapterError`, and `ProviderAdapterErrorKind`.
- Produces `ProviderKind::encode_request(&self, request, credential)` and `ProviderKind::decode_response(&self, requested_model, status, body)` dispatch points; provider arms may initially return the stable `MalformedResponse` error until their task lands.

- [x] Write RED tests for exact model/system/input/schema/token/idempotency bounds, top-level object requirements, schema-name syntax, request/response/output byte caps, and redacted `Debug`/`Display`.
- [x] Implement canonical types with custom `Debug` for `ProviderHeader`, `EncodedProviderRequest`, and `ProviderAdapterError`.
- [x] Implement validation helpers that canonicalize JSON through `serde_json::to_vec`, enforce byte bounds before provider mapping, and keep the default recursion limit enabled during response parsing.
- [x] Export `pub mod provider` from `src/content/mod.rs` and add explicit provider dispatch modules.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_adapters canonical -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Commit and push: `feat: define ai provider protocol`.

### Task 2: Deterministic provider request encoders

**Files:**
- Create: `src/content/provider/anthropic.rs`
- Create: `src/content/provider/openai_responses.rs`
- Create: `src/content/provider/openai_chat.rs`
- Create: `src/content/provider/gemini.rs`
- Modify: `src/content/provider/mod.rs`
- Modify: `tests/ai_provider_adapters.rs`
- Create: `tests/fixtures/ai-provider/v1/anthropic-request.json`
- Create: `tests/fixtures/ai-provider/v1/openai-responses-request.json`
- Create: `tests/fixtures/ai-provider/v1/openai-chat-request.json`
- Create: `tests/fixtures/ai-provider/v1/gemini-request.json`

**Interfaces:**
- Consumes the validated canonical request and `SecretString` credential.
- Produces provider paths, secret/public headers, and a JSON body capped at 1 MiB.

- [x] Add RED fixture assertions for all four exact JSON shapes, header names/sensitivity, public idempotency headers, Gemini model path encoding, and credential absence from path/body/debug.
- [x] Implement Anthropic Messages mapping to `/v1/messages`, `output_config.format`, and fixed `anthropic-version`.
- [x] Implement OpenAI Responses mapping to `/v1/responses`, `instructions`, user input text, `max_output_tokens`, and strict `text.format` JSON schema.
- [x] Implement OpenAI Chat Completions mapping to `/v1/chat/completions`, system/user messages, `max_completion_tokens`, and strict `response_format.json_schema`.
- [x] Implement Gemini mapping to one encoded model path segment, `systemInstruction`, `contents`, and JSON `generationConfig` using secret `x-goog-api-key` rather than a query key.
- [x] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_adapters request -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [x] Commit and push: `feat: encode ai provider requests`.

### Task 3: Hostile response decoders and normalization

**Files:**
- Modify: `src/content/provider/anthropic.rs`
- Modify: `src/content/provider/openai_responses.rs`
- Modify: `src/content/provider/openai_chat.rs`
- Modify: `src/content/provider/gemini.rs`
- Modify: `src/content/provider/mod.rs`
- Modify: `src/content/provider/validation.rs`
- Modify: `tests/ai_provider_adapters.rs`
- Create: `tests/fixtures/ai-provider/v1/anthropic-response.json`
- Create: `tests/fixtures/ai-provider/v1/openai-responses-response.json`
- Create: `tests/fixtures/ai-provider/v1/openai-chat-response.json`
- Create: `tests/fixtures/ai-provider/v1/gemini-response.json`

**Interfaces:**
- Consumes requested model, HTTP status, and bounded response bytes.
- Produces a top-level JSON object, normalized finish reason, optional usage counts, and bounded model label.

- [ ] Add RED success fixtures for output extraction, model fallback, usage, zero usage, and each provider's stop/length/content-filter/tool/unknown finish mappings.
- [ ] Add RED hostile fixtures for unknown additive fields, missing/multiple output text, malformed JSON, non-object output, excessive nesting, response/output size, and credential-like error bodies.
- [ ] Implement status-first provider-safe error classification with no retained raw body.
- [ ] Implement typed provider response structs that ignore unknown fields and fail closed on required output facts.
- [ ] Parse extracted text with the default Serde recursion limit, require a top-level object, enforce the 512 KiB canonical output cap, and normalize usage/model/finish reason.
- [ ] Verify:

```bash
cargo fmt --check
cargo test --locked --test ai_provider_adapters response -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
git diff --check
```

- [ ] Commit and push: `feat: decode ai provider responses`.

### Task 4: Full gates, source-boundary audit, report, and CI

**Files:**
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/ai-provider-adapters-v1-report.md`

**Interfaces:**
- Produces exact local/CI evidence and points the next AI slice at provider storage plus SSRF-safe transport.

- [ ] Run a source-boundary scan proving provider wire keys and credential header names do not escape `src/content/provider`, its integration test, or its fixtures.
- [ ] Run fresh full gates:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

- [ ] Record exact test counts, redaction evidence, fixture inventory, commits, explicit remaining work, and existing advisories in `.superpowers/sdd/ai-provider-adapters-v1-report.md`.
- [ ] Update `tasks/plan.md` to name provider storage and SSRF-safe execution as the next AI slice. Keep the AI/plugin/MCP todo items unchecked because no user-visible plugin operation exists yet.
- [ ] Commit and push: `test: verify ai provider adapters`.
- [ ] Monitor the triggered CI only for concrete failures; append the successful run to the report and push a `[skip ci]` closeout.

## Plan self-review

- Spec coverage: canonical validation/redaction, four request shapes, four response shapes, errors, bounds, fixtures, source containment, full gates, report, push, and CI each map to one task.
- Dependency order: canonical types first, encoders second, decoders third, delivery evidence last.
- Type consistency: `ProviderKind`, `StructuredGenerationRequest`, `EncodedProviderRequest`, `StructuredGenerationResponse`, `TokenUsage`, `FinishReason`, and error kinds are identical across tasks.
- Security: credentials are secret headers; provider bodies and model output never enter errors; byte/token/recursion boundaries are testable.
- DDIA/evolution: canonical internal records isolate additive provider wire evolution; no second business model or persisted derived view is introduced.
- Placeholder scan: no TBD, unspecified error handling, open review loop, subagent dispatch, storage claim, route claim, or UI claim remains.
