# AI Provider Adapters v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- Added one canonical structured-generation request and response contract under `src/content/provider`.
- Added deterministic non-streaming request encoders for Anthropic Messages-compatible, OpenAI Responses, OpenAI Chat Completions-compatible, and Google Gemini APIs.
- Added typed hostile response decoders that normalize structured JSON output, model label, optional token usage, and finish reason.
- Added status-first authentication, rate-limit, timeout, rejected-request, and upstream-failure classification without retaining provider error bodies.
- Added explicit request, response, output, model, schema, token, idempotency-key, and JSON recursion boundaries.
- Added secret/public header separation and custom redacted `Debug` implementations for canonical requests, output schemas, encoded provider requests, structured responses, and provider errors.
- Added eight committed protocol fixtures and thirteen integration tests.

This is a real protocol layer, not a provider-name registry or empty trait. It does not yet send network requests or expose an AI feature to users.

## Delivery commits

- `791a956 docs: plan ai provider adapters`
- `811e90b feat: define ai provider protocol`
- `191897e feat: encode ai provider requests`
- `02a689e feat: decode ai provider responses`

The verification commit and remote CI run are appended after push.

## Protocol contracts

| Provider kind | Path | Structured-output mapping | Authentication |
| --- | --- | --- | --- |
| `ANTHROPIC_MESSAGES` | `/v1/messages` | `output_config.format` JSON schema | secret `x-api-key` header |
| `OPENAI_RESPONSES` | `/v1/responses` | strict `text.format` JSON schema | secret bearer `authorization` header |
| `OPENAI_CHAT_COMPLETIONS` | `/v1/chat/completions` | strict `response_format.json_schema` | secret bearer `authorization` header |
| `GOOGLE_GEMINI` | `/v1beta/models/{encoded}:generateContent` | JSON `generationConfig.responseJsonSchema` | secret `x-goog-api-key` header |

Gemini model identifiers are encoded as one path segment; slash, query, and fragment characters cannot alter routing. No credential is placed in a URL query or request body.

The committed parent specification is the authority for this slice. Attempts to re-check current official Web documentation were blocked in the local environment by a Vercel 403, an Anthropic region redirect, and Google documentation timeouts. Production activation therefore remains gated on the later transport slice's live provider contract probes; this report does not claim that network execution has been validated.

## Security and redaction evidence

- Canonical request validation requires bounded model/system/input/schema/token/idempotency values and top-level JSON objects.
- Encoded request bodies are capped at 1 MiB; provider response bodies at 2 MiB; decoded structured output at 512 KiB.
- Serde's default recursion limit remains enabled. An excessive-depth model output fixture fails closed.
- Model output is parsed as JSON data and must be a top-level object. It is never evaluated, rendered, used as SQL, or passed to a shell.
- Provider errors retain only provider kind and stable error kind. Credential-like and prompt-like upstream bodies do not appear in `Debug`, `Display`, or error chains.
- `SecretString` owns credential header values. Encoded request debug output contains header metadata, `[REDACTED]`, and body byte count only.
- Canonical request/response debug output contains byte counts and safe metadata, not prompt, schema, input, idempotency key, or model output.
- Unknown provider response fields are ignored for additive compatibility; missing or ambiguous required output facts fail closed.

## Fixture inventory

- `anthropic-request.json` / `anthropic-response.json`
- `openai-responses-request.json` / `openai-responses-response.json`
- `openai-chat-request.json` / `openai-chat-response.json`
- `gemini-request.json` / `gemini-response.json`

The fixtures cover exact request shapes, unknown additive fields, model labels, token usage, structured output, and the primary successful finish reason. Tests mutate them to cover missing/zero usage, model fallback, stop/length/content-filter/tool/unknown finish families, missing/multiple outputs, malformed/non-object/oversized/deep JSON, invalid model labels, and provider-safe non-2xx errors.

## Fresh deterministic verification

- `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo test --locked --test ai_provider_adapters -- --nocapture`: 13 tests passed.
- `cargo test --locked --all-features`: 465 executed tests passed; one opt-in IT之家 live RSS smoke remained ignored by design.
- `git diff --check`: passed.
- Source-boundary scan: provider wire keys and credential header names appear only in `src/content/provider`, the focused integration test, and its fixtures. Test credential/prompt markers do not appear in production provider source.

No Cargo dependency or lockfile changed. No database, HTTP route, frontend, setup, auth, Feed transaction, lifecycle dispatcher, MCP transport, or plugin runtime code changed.

## Existing advisories

- `proc-macro-error2 v2.0.1` remains the tracked future-incompatibility advisory through the SeaORM dependency chain.
- This slice does not change the existing release-build dead-code or Vite chunk-size advisories.

## Explicitly remaining

- Persist provider records, immutable provider kind, encrypted credentials, capability flags, quota and cost policy.
- Implement SSRF-safe HTTPS execution with endpoint validation, DNS/IP policy, peer pinning, redirect policy, timeouts, response streaming bounds, and retry metadata.
- Add provider administration API/UI without exposing secrets.
- Add the signed bundled `raindrop.ai-content` Wasm Component, WIT ABI, manifest verification, sandbox, and capability broker.
- Add content jobs, attempts, artifacts, lifecycle fan-out, Reader sidecar, translation/summary UI, and automatic rules.
- Add MCP client bindings, read-only enrichment, failure policy, audit, recursion protection, and Raindrop MCP server reuse.
- Add streaming provider decoding before production provider configuration is enabled.
