# Raindrop AI Provider Adapters v1 Design

Date: 2026-07-18

Status: internally approved implementation slice

Parent specification: `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`

## Objective

Deliver the canonical structured-generation protocol layer used by the official `raindrop.ai-content` plugin host. The layer must encode real non-streaming requests and decode real non-streaming responses for:

- `ANTHROPIC_MESSAGES`
- `OPENAI_RESPONSES`
- `OPENAI_CHAT_COMPLETIONS`
- `GOOGLE_GEMINI`

All four adapters consume one bounded `StructuredGenerationRequest` and return one `StructuredGenerationResponse`. Provider-specific JSON and authentication headers stop at this module boundary. The future content orchestrator, Wasm host, job worker, artifact store, MCP broker, API, and UI must never branch on provider-specific DTOs.

Success means deterministic fixtures prove each adapter's request shape, output extraction, JSON validation, usage and finish-reason normalization, provider-safe error classification, and secret/body redaction.

## Assumptions and scope

- The parent AI plugin specification is authoritative and decision-complete; the user delegated review and confirmation to the main Agent.
- This slice implements the non-streaming structured-output path. Streaming transport decoding remains additive and must be completed before any provider is exposed through production configuration.
- Provider storage, endpoint policy, DNS/IP pinning, redirects, quotas, concurrency, retries, HTTP execution, admin APIs, Wasmtime, jobs, artifacts, MCP, and Reader UI are separate slices.
- A later provider core supplies an already-authorized model identifier and credential. This module never loads a secret from configuration or a database.
- No real model request is sent in tests. Protocol fixtures are deterministic and contain placeholder credentials only inside test construction.
- No new Rust dependency is required. Existing `http`, `secrecy`, `serde`, `serde_json`, `thiserror`, `url`, and `zeroize` are sufficient.

## Trust boundaries and abuse cases

| Boundary | Abuse case | Control |
| --- | --- | --- |
| Canonical request | Huge prompt/schema or invalid model identifier exhausts memory or creates ambiguous paths | Exact UTF-8 byte limits, control-character rejection, object-only input/schema, bounded token ceiling |
| Credential | Secret appears in `Debug`, error, serialized body, URL query, or fixture snapshot | `SecretString`, secret header wrapper, header-only authentication, redacted `Debug`, no query-key authentication |
| Provider response | Oversized or deeply nested JSON consumes memory/stack | Byte cap before parsing; default Serde recursion limit remains enabled |
| Model output | Output text contains instructions, HTML, SQL, or non-JSON data | Parse only as JSON; require a top-level object; never execute or render in this module |
| Error response | Upstream body echoes prompt, key, URL, or internal stack | Classify by HTTP status and discard the raw body from public/debug errors |
| Compatible endpoint | Missing fields or variant response silently becomes corrupt artifact | Typed provider decoders fail closed on required output fields; unknown fields remain additive |

## Canonical module boundary

Create focused files under `src/content/provider/`:

```text
mod.rs                    public internal facade and ProviderKind dispatch
types.rs                  canonical request/response/error/header types
validation.rs             shared byte, JSON-object, model and token bounds
anthropic.rs              Anthropic Messages request/response mapping
openai_responses.rs       OpenAI Responses request/response mapping
openai_chat.rs            OpenAI Chat Completions request/response mapping
gemini.rs                 Google Gemini request/response mapping
```

`src/content/mod.rs` exposes `pub mod provider`; no other module exports provider-specific request or response structs.

## Canonical types

```rust
pub enum ProviderKind {
    AnthropicMessages,
    OpenAiResponses,
    OpenAiChatCompletions,
    GoogleGemini,
}

pub struct OutputSchema {
    pub name: String,
    pub schema: serde_json::Value,
}

pub struct StructuredGenerationRequest {
    pub model: String,
    pub system_instruction: String,
    pub untrusted_input: serde_json::Value,
    pub output_schema: OutputSchema,
    pub max_output_tokens: u32,
    pub idempotency_key: String,
}

pub struct EncodedProviderRequest {
    pub path: String,
    pub headers: Vec<ProviderHeader>,
    pub body: Vec<u8>,
}

pub struct StructuredGenerationResponse {
    pub output: serde_json::Value,
    pub finish_reason: FinishReason,
    pub usage: TokenUsage,
    pub model_label: String,
}

pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCall,
    Other,
}
```

`ProviderHeader` stores a validated `HeaderName` and either a public `HeaderValue` or a `SecretString`. Its `Debug` output prints the name and `[REDACTED]` for secret values. `EncodedProviderRequest::Debug` prints only provider path, header metadata, and body byte count; it never prints request JSON.

## Shared validation and limits

| Value | Contract |
| --- | --- |
| model | 1..=200 UTF-8 bytes; no ASCII control characters |
| system instruction | at most 64 KiB |
| untrusted input | top-level object; canonical serialized size at most 512 KiB |
| schema name | 1..=64 ASCII alphanumeric, `_`, or `-`; first byte alphanumeric |
| output schema | top-level object; canonical serialized size at most 64 KiB |
| output tokens | 1..=16,384 |
| idempotency key | 1..=200 UTF-8 bytes; visible ASCII only |
| encoded request body | at most 1 MiB |
| provider response body | at most 2 MiB before JSON parse |
| decoded structured output | top-level object; canonical serialized size at most 512 KiB |

Validation returns stable `ProviderAdapterErrorKind` values and never includes raw model input, schema, credential, response body, or parsed output in `Display`/`Debug`.

## Request mappings

### Anthropic Messages

- Path: `/v1/messages`
- Headers: `content-type: application/json`, `accept: application/json`, secret `x-api-key`, public `anthropic-version: 2023-06-01`, and public `idempotency-key`.
- Body fields: `model`, `max_tokens`, `system`, one user `messages` item whose text is the canonical untrusted-input JSON, and `output_config.format = { type: "json_schema", schema }`.
- Schema name is not sent because the Messages structured-output format consumes the schema directly; it remains part of the canonical request and later audit identity.

### OpenAI Responses

- Path: `/v1/responses`
- Headers: JSON content/accept, secret `authorization: Bearer ...`, and public `idempotency-key`.
- Body fields: `model`, `instructions`, `input` containing one user input-text item with canonical untrusted JSON, `max_output_tokens`, and `text.format = { type: "json_schema", name, schema, strict: true }`.

### OpenAI Chat Completions

- Path: `/v1/chat/completions`
- Headers: JSON content/accept, secret `authorization: Bearer ...`, and public `idempotency-key`.
- Body fields: `model`, system/user messages, `max_completion_tokens`, and `response_format = { type: "json_schema", json_schema: { name, schema, strict: true } }`.

### Google Gemini

- Path: `/v1beta/models/{percent-encoded-model}:generateContent`; the model is encoded as one URL path segment so `/`, `?`, and `#` cannot alter routing.
- Headers: JSON content/accept, secret `x-goog-api-key`, and public `x-goog-request-id`. The credential never appears in the query string.
- Body fields: `systemInstruction.parts[0].text`, one user `contents` item with canonical untrusted JSON, and `generationConfig = { maxOutputTokens, responseMimeType: "application/json", responseJsonSchema: schema }`.

## Response mappings

- Non-2xx responses are classified before provider JSON decoding: `401/403` authentication, `408/504` timeout, `429` rate limited, other `4xx` rejected, and `5xx` upstream failure. Raw bodies are discarded.
- Anthropic extracts exactly one non-empty `content` block whose `type` is `text`; usage reads `input_tokens` and `output_tokens`; stop reasons normalize by stable string.
- OpenAI Responses extracts exactly one non-empty `output_text` content item from a message output; usage reads `input_tokens` and `output_tokens`; completed/incomplete status and incomplete detail normalize the finish reason.
- OpenAI Chat Completions requires a first choice with string `message.content`; usage reads `prompt_tokens` and `completion_tokens`; `finish_reason` maps directly.
- Gemini requires a first candidate with exactly one non-empty text part; usage reads `promptTokenCount` and `candidatesTokenCount`; `finishReason` maps from the documented uppercase enum family.
- Missing usage is represented by `None`; zero is preserved as `Some(0)`. Unknown finish reasons map to `Other` rather than breaking compatible endpoints.
- Extracted text is parsed as JSON under the default recursion limit, must be a top-level object, and must fit the decoded-output cap.
- The returned model label uses the response model/version when present and otherwise the requested model. It is bounded to 200 bytes and redacted on corruption.

## Error contract

```rust
pub enum ProviderAdapterErrorKind {
    InvalidRequest,
    RequestTooLarge,
    ResponseTooLarge,
    Authentication,
    RateLimited,
    Timeout,
    Rejected,
    Upstream,
    MalformedResponse,
    OutputSchemaInvalid,
}
```

`ProviderAdapterError` exposes only `kind()` and the provider kind. `Display` contains a stable sentence. `Debug` contains the provider and kind only. There is no `source` that can retain provider body JSON.

## Commands

```bash
cargo fmt --check
cargo test --locked --test ai_provider_adapters -- --nocapture
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

## Testing strategy

- Canonical validation tests cover every exact lower/upper boundary, object-only JSON, control characters, invalid schema names, token ceilings, and canonical-size enforcement.
- Request fixtures assert exact paths, header names/sensitivity, JSON shapes, canonical untrusted input, strict structured-output fields, Gemini path encoding, and absence of credentials in URL/body/debug text.
- Response fixtures cover success, missing optional usage, zero usage, every normalized finish family, unknown additive fields, missing required output, multiple output candidates, non-object output, malformed JSON, default recursion rejection, and byte caps.
- Error tests inject credential-like strings and prompt fragments into upstream bodies and assert no formatted error/request contains them.
- Source-boundary tests ensure the four provider modules are the only files containing provider wire field names.

## Boundaries

- Always: treat provider responses and model output as hostile; validate before returning; keep credentials in secret headers; keep provider DTOs inside `src/content/provider`.
- Internally self-review before implementation because the user delegated confirmation to the main Agent.
- Never: send a real model request, log a credential or body, put credentials in a URL, disable Serde recursion limits, expose provider DTOs to plugin/job/API/UI code, or claim the AI plugin is complete after this slice.

## Success criteria

- All four provider kinds encode deterministic structured-generation requests from the same canonical input.
- All four provider kinds decode deterministic structured JSON responses into the same canonical response.
- Credential, prompt, schema, untrusted input, provider body, and model output do not appear in `Debug`, `Display`, or error chains.
- Oversized, malformed, ambiguous, or non-object outputs fail closed with stable kinds.
- No runtime dependency is added and the complete Rust suite remains green.

## Internal self-review

- DDIA/evolution: provider wire formats are derived adapters; canonical types are the internal record. Unknown response fields remain additive, while missing required facts fail closed.
- API/interface: provider-specific behavior is contained behind one typed dispatch surface; future streaming and transport are additive methods rather than changes to canonical semantics.
- Security: no real network in tests, no query credentials, explicit byte/token caps, hostile model output parsing, redacted errors, and no secret-bearing `Debug`.
- Scope: storage, endpoint policy, transport, provider administration, Wasmtime, jobs, artifacts, lifecycle, MCP, and UI remain named follow-up slices rather than empty interfaces.
- Open questions: none for this slice.
