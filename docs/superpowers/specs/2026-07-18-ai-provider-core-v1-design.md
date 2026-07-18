# Raindrop AI Provider Core v1 Design

Date: 2026-07-18

Status: internally approved implementation slice

Parent specifications:

- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/superpowers/specs/2026-07-18-ai-provider-adapters-v1-design.md`

## Objective

Turn the completed provider protocol adapters into a real provider core without exposing an AI feature prematurely. This slice adds the authoritative provider record, encrypted credential storage, immutable protocol kind, typed capability/quota/cost policy, an SSRF-safe HTTPS transport, and one redacted `ProviderClient` that executes the existing canonical structured-generation contract.

Success means an authorized provider binding can be loaded from SQLite, PostgreSQL, or MySQL, decrypted only inside provider core, encoded by one of the four existing adapters, sent through a DNS-pinned transport, and decoded into the existing canonical response. No provider secret, endpoint, request body, response body, prompt, schema, or model output may appear in logs, errors, `Debug`, API DTOs, or database verification output.

This is still an internal core slice. It does not add provider administration HTTP routes or UI, content jobs, artifacts, the official Wasm plugin, lifecycle dispatch, MCP, streaming generation, or live provider credentials.

## Assumptions and delegated review

- The user delegated design review and confirmation to the main Agent, so this specification is internally self-reviewed instead of waiting for an external approval checkpoint.
- Provider records may be instance-scoped or owned by one user. Authorization to create or mutate them remains a future API/service concern; repository methods require an explicit `ProviderScope` and never infer administrator authority.
- A provider credential is an opaque secret string from 1 through 8,192 UTF-8 bytes. Provider-specific credential syntax stays inside the adapter/transport boundary.
- Provider endpoints are HTTPS only. Compatible endpoints may use a fixed path prefix, but never credentials, query parameters, fragments, dot segments, or encoded path separators.
- Redirects are denied. This prevents a credential-bearing request from being forwarded to a second authority and makes the redirect policy unambiguous.
- Streaming remains unsupported in v1. Persisted capability data must reject `supports_streaming = true` until a later adapter and transport slice implements it.
- No real provider request is sent by deterministic tests. Production transport behavior is proved with injected DNS/executor/body fakes; a credentialed live contract probe remains a release gate before user-visible provider configuration is enabled.

## Tech stack and commands

- Rust 1.94.0, edition 2024.
- SeaORM/sea-orm-migration for SQLite, PostgreSQL, and MySQL.
- Existing `reqwest`, `rustls`, `hickory-resolver`, `async-compression`, `http`, `httpdate`, `url`, `base64`, `secrecy`, `zeroize`, and `blake3` dependencies.
- Add a direct exact dependency on `ring = "=0.17.14"`; it is already locked transitively through rustls and supplies audited AES-256-GCM plus the operating-system CSPRNG.

Verification commands:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test ai_provider_secret -- --nocapture
cargo test --locked --test ai_provider_storage -- --nocapture --test-threads=1
cargo test --locked --test ai_provider_transport -- --nocapture
cargo test --locked --all-features
git diff --check
```

PostgreSQL/MySQL contract tests use the existing `RAINDROP_TEST_POSTGRES_URL` and `RAINDROP_TEST_MYSQL_URL` variables and run explicitly in CI before the environment variables are removed for the deterministic full suite.

## Project structure

```text
src/config/model.rs                         optional encrypted-provider key entries
src/config/loader.rs                        env/TOML precedence and fail-closed key validation
src/db/migration/ai_providers.rs            portable authoritative provider table
src/db/entities/ai_provider.rs              SeaORM persistence model only
src/content/provider/model.rs               scope, endpoint, metadata, capability and policy types
src/content/provider/secret.rs              versioned AES-256-GCM keyring and envelope
src/content/provider/repository.rs          scoped CRUD and decrypted binding load
src/content/provider/transport/mod.rs       transport trait and staged orchestration
src/content/provider/transport/dns.rs       resolver, address approval and pin set
src/content/provider/transport/http.rs      reqwest executor, headers and peer verification
src/content/provider/transport/body.rs      bounded streaming and decompression
src/content/provider/transport/types.rs     response, timeout and redacted error types
src/content/provider/client.rs              adapter + binding + transport composition
tests/ai_provider_secret.rs                 encryption, rotation, AAD and redaction contract
tests/ai_provider_storage.rs                three-database schema/repository contract
tests/ai_provider_transport.rs              client composition and public error contract
docs/ai-providers.md                        operator key and endpoint policy
```

Provider-specific wire DTOs remain confined to their existing adapter files. The new files consume `ProviderKind`, `EncodedProviderRequest`, `StructuredGenerationRequest`, and `StructuredGenerationResponse`; they do not introduce a second request/response business model.

## Trust boundaries and abuse cases

| Boundary | Abuse case | Control |
| --- | --- | --- |
| Configuration | A malformed key, duplicate key ID, or secret-bearing parse error reaches logs | Exact key-entry grammar, full key-length validation, `SecretString`, redacted config errors |
| Database | Plain credential is stored, copied into metadata, or substituted between providers | AES-256-GCM envelope; provider ID and immutable kind in AAD; metadata type has no credential field |
| Repository scope | A user loads another user's provider or an instance mutation hits a user record | Explicit `ProviderScope` predicates; invisible resources return `NotFound`; no administrator bypass |
| Endpoint | Credentials/query/fragment/path traversal create request smuggling or routing ambiguity | HTTPS-only normalized base URL; forbidden userinfo/query/fragment/dot/encoded separators; bounded length |
| DNS | Private address, mixed public/private answers, rebinding, or too many answers bypasses SSRF policy | Resolve once; reject if any address is denied; cap at 16; pin all approved addresses into reqwest |
| Connection | Client connects to an address outside the approved DNS result | Require `response.remote_addr()` and exact membership in approved socket addresses |
| Redirect | Upstream forwards authorization to another host | reqwest redirects disabled; every 3xx is a typed `RedirectDenied` without reading a body |
| Response | Slow/huge/compressed/deep hostile output consumes resources | staged deadlines, idle timeout, compressed and decoded caps, expansion-ratio cap, existing Serde recursion limit |
| Errors | reqwest URL, provider body, endpoint, secret, prompt, schema, or output escapes | URL-stripped sources, stable error kinds, custom `Debug`/`Display`, body discarded on non-2xx |

## Configuration keyring

Provider secrets use a dedicated keyring and are never derived from the session secret. Reusing the session secret would couple login-token rotation to AI credential availability and is rejected by this design.

The environment variable is:

```text
RAINDROP_PROVIDER_SECRET_KEYS=primary:<base64url-32-bytes>,previous:<base64url-32-bytes>
```

The TOML equivalent is:

```toml
provider_secret_keys = [
  "primary:<base64url-32-bytes>",
  "previous:<base64url-32-bytes>",
]
```

Rules:

- Environment replaces the whole TOML list; individual entries are not merged.
- Configuration is optional so existing non-AI installations continue to start. Provider secret creation/decryption requires a constructed keyring and otherwise returns `KeyUnavailable`.
- The first entry is the active encryption key. Remaining entries are decryption-only rotation keys.
- Key IDs are 1 through 32 visible ASCII bytes, begin with an alphanumeric byte, and contain only alphanumeric, `_`, or `-`.
- Key material is URL-safe base64 without padding and decodes to exactly 32 bytes.
- Key IDs are unique. Duplicate key material under different IDs is rejected to avoid ambiguous rotation state.
- `RuntimeConfig` and keyring `Debug` show only entry count and non-secret key IDs; key material is never exposed.
- Operators must retain old entries until all envelopes using them have been rewrapped or provider credentials have been re-entered.

## Secret envelope

The persisted `encrypted_secret` is versioned text:

```text
rdsec1.<key-id>.<nonce-base64url>.<ciphertext-and-tag-base64url>
```

- Algorithm: AES-256-GCM from ring.
- Nonce: 96 random bits from `ring::rand::SystemRandom`, generated independently for every encryption.
- Tag: the 128-bit GCM tag appended to ciphertext by ring.
- Associated data is exact bytes:

```text
raindrop.ai-provider-secret.v1\0<provider-id>\0<provider-kind-storage-value>
```

- Decryption selects the envelope key ID, authenticates AAD, returns a `SecretString`, and zeroizes temporary decoded key/plaintext buffers where their owning types permit it.
- Changing provider ID or kind, changing any envelope byte, using an unknown key ID, or supplying malformed base64 fails closed as `DecryptFailed` without distinguishing the precise corruption to callers.
- Re-encrypting the same plaintext produces a different envelope because the nonce changes.
- The envelope and ciphertext may appear only inside the SeaORM persistence model. Public/provider-core metadata and formatted errors never contain them.

## Authoritative provider record

Create `ai_providers` with:

| Column | Contract |
| --- | --- |
| `id` | UUID string primary key |
| `owner_user_id` | nullable user FK; `NULL` means instance scope; user deletion cascades |
| `display_name` | 1..=80 UTF-8 bytes after trim; no ASCII controls |
| `kind` | one of the four stable storage values; immutable after insert |
| `endpoint` | normalized HTTPS base URL, at most 2,048 bytes |
| `model` | 1..=200 UTF-8 bytes; no ASCII controls |
| `encrypted_secret` | versioned ciphertext envelope only |
| `supports_usage` | canonical capability flag |
| `supports_idempotency` | canonical capability flag |
| `supports_streaming` | stored for additive evolution; must be false in v1 |
| `max_concurrency` | 1..=64 |
| `requests_per_minute` | nullable 1..=1,000,000 |
| `max_input_tokens_per_request` | 1..=1,048,576 |
| `max_output_tokens_per_request` | 1..=16,384 |
| `input_cost_micros_per_million_tokens` | nullable 0..=1,000,000,000,000 |
| `output_cost_micros_per_million_tokens` | nullable 0..=1,000,000,000,000 |
| `max_cost_micros_per_request` | nullable 0..=1,000,000,000,000 |
| `is_enabled` | disabled providers cannot produce a binding |
| `revision` | non-negative optimistic-concurrency revision |
| `created_at`, `updated_at` | operational UTC timestamps with the existing MySQL microsecond contract |

Indexes:

- `idx_ai_providers_owner_enabled(owner_user_id, is_enabled)` for scoped listing/binding lookup.
- `idx_ai_providers_kind(kind)` for administration and migration diagnostics.

Names are not unique. Provider IDs are the stable references used by future plugin configuration and jobs; allowing duplicate display names avoids a cross-database nullable-scope uniqueness workaround and does not weaken identity.

The table is the record system for provider configuration. Capability and policy columns are normalized canonical facts, not provider-specific JSON. Future policy fields are additive columns; provider-specific wire configuration must not leak into this table.

## Canonical provider types

`ProviderKind` gains exact storage/display helpers and default endpoint mapping:

```rust
pub const fn as_storage(self) -> &'static str;
pub fn from_storage(value: &str) -> Result<Self, ProviderCoreError>;
pub const fn default_endpoint(self) -> &'static str;
```

New core types:

```rust
pub enum ProviderScope {
    Instance,
    User(String),
}

pub struct ProviderCapabilities {
    pub supports_usage: bool,
    pub supports_idempotency: bool,
    pub supports_streaming: bool,
}

pub struct ProviderPolicy {
    pub max_concurrency: u16,
    pub requests_per_minute: Option<u32>,
    pub max_input_tokens_per_request: u32,
    pub max_output_tokens_per_request: u32,
    pub input_cost_micros_per_million_tokens: Option<u64>,
    pub output_cost_micros_per_million_tokens: Option<u64>,
    pub max_cost_micros_per_request: Option<u64>,
}

pub struct CreateProvider {
    pub scope: ProviderScope,
    pub display_name: String,
    pub kind: ProviderKind,
    pub endpoint: Option<String>,
    pub model: String,
    pub credential: SecretString,
    pub capabilities: ProviderCapabilities,
    pub policy: ProviderPolicy,
    pub is_enabled: bool,
}

pub struct UpdateProvider {
    pub expected_revision: u64,
    pub display_name: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential: Option<SecretString>,
    pub capabilities: Option<ProviderCapabilities>,
    pub policy: Option<ProviderPolicy>,
    pub is_enabled: Option<bool>,
}
```

`UpdateProvider` deliberately has no kind field. `ProviderMetadata` contains ID, scope, name, kind, endpoint, model, capabilities, policy, enabled state, revision, and timestamps, but no encrypted or decrypted credential. Its custom `Debug` redacts endpoint and model because compatible endpoints/model labels can contain tenant-specific operational details.

`ProviderBinding` is internal execution state: metadata plus a decrypted `SecretString`. Its `Debug` prints only provider ID, kind, enabled state, revision, and `[REDACTED]` markers.

## Repository contract

`ProviderRepository` owns a database connection and `ProviderSecretKeyring`.

```rust
pub async fn create(&self, input: CreateProvider) -> Result<ProviderMetadata, ProviderCoreError>;
pub async fn get(&self, id: &str, scope: &ProviderScope) -> Result<ProviderMetadata, ProviderCoreError>;
pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<ProviderMetadata>, ProviderCoreError>;
pub async fn update(&self, id: &str, scope: &ProviderScope, patch: UpdateProvider) -> Result<ProviderMetadata, ProviderCoreError>;
pub async fn load_enabled_binding(&self, id: &str, user_id: &str) -> Result<ProviderBinding, ProviderCoreError>;
```

Rules:

- User scope requires a valid UUID and an existing, non-disabled user at create time.
- `get` and `update` require exact scope. A different owner and an unknown ID both return `NotFound`.
- `list_for_user` returns enabled and disabled instance providers plus the user's own providers. It sorts the bounded result in Rust by Unicode-lowercased display name then ID so database collation differences cannot change the contract; it never returns another user's records.
- `load_enabled_binding` admits an instance provider or the requesting user's provider. Disabled, invisible, missing, corrupt, or undecryptable records return stable non-secret errors.
- Create validates all fields before opening its short transaction, generates the UUID before encryption, binds ciphertext AAD to that UUID/kind, and inserts once.
- Update loads the exact scoped row, validates a full prospective record, encrypts a replacement credential only when supplied, and performs `WHERE id = ? AND revision = ?` optimistic update. Zero affected rows after a successful scoped read returns `RevisionConflict`.
- Update preserves kind, owner, ID, creation time, and existing ciphertext when no replacement credential is supplied.
- No repository method deletes a provider in this slice. Future job/config references require a separate disable/retention design before deletion is exposed.

## Endpoint contract

`ProviderEndpoint` owns the complete normalized URL privately and implements neither `Display` nor Serde.

- Input defaults to `ProviderKind::default_endpoint()` when omitted.
- Scheme is exactly `https`.
- Username/password, query, fragment, empty host, control characters, spaces, and URL length above 2,048 bytes are rejected.
- Literal IP hosts are classified immediately by the same public-only address policy used by Feed security; private/reserved literals are rejected before storage.
- A path prefix is allowed. The normalized endpoint always ends in `/`.
- Literal or percent-encoded `.`/`..` segments and percent-encoded `/` or `\\` are rejected.
- The adapter path must begin with one `/`, contain no query/fragment/backslash/control character, and contain no dot segment. Joining trims the one leading slash and resolves relative to the endpoint directory, so `https://gateway.example/api/` plus `/v1/responses` becomes `https://gateway.example/api/v1/responses` without changing authority.
- `Debug` contains only scheme, canonical host, effective port, and path segment count.

Default endpoints:

| Kind | Endpoint |
| --- | --- |
| Anthropic Messages | `https://api.anthropic.com/` |
| OpenAI Responses | `https://api.openai.com/` |
| OpenAI Chat Completions | `https://api.openai.com/` |
| Google Gemini | `https://generativelanguage.googleapis.com/` |

## SSRF-safe HTTPS transport

The transport accepts only `ProviderEndpoint` plus an adapter-produced `EncodedProviderRequest`; callers cannot provide a raw URL or arbitrary method.

Production behavior:

1. Compute a 90-second total deadline.
2. Join and revalidate the adapter path against the stored endpoint.
3. Resolve A and AAAA within 3 seconds. Literal IPs skip DNS.
4. Reject zero answers, more than 16 answers, or the whole set when any address is private, loopback, link-local, documentation, transition-special, multicast, or otherwise denied by `AddressPolicy::public_only()`.
5. Deduplicate approved addresses and attach the endpoint's effective port.
6. Build a per-request reqwest client with `no_proxy`, no automatic redirects, no automatic decompression, a 5-second connect timeout, a 20-second first-byte/header deadline, a 10-second body-idle timeout, and the remaining total request timeout. Use `resolve_to_addrs` so the connection can use only the approved sockets while TLS still verifies the original hostname.
7. Convert adapter headers. Reject invalid header values and any attempt to set `Host`, `Content-Length`, `Connection`, `Transfer-Encoding`, `Upgrade`, `Proxy-Authorization`, or `Proxy-Authenticate`. Secret header values exist only in the request builder's short-lived memory.
8. Send exactly one POST. Require `remote_addr()` and exact membership in the approved set.
9. Parse at most one valid `Retry-After` value relative to response receipt time.
10. Deny every 3xx as `RedirectDenied` without reading the body.
11. For non-2xx, do not read or retain the response body; return status plus retry metadata so the adapter can classify it safely.
12. For 2xx, stream the compressed body with a 2 MiB compressed cap and idle deadline. Support identity, gzip, Brotli, and zlib-wrapped deflate with one encoding only, a 2 MiB decoded cap, and a 100x expansion-ratio cap.
13. Return status, bounded decoded bytes, and optional retry deadline. `Debug` reports body byte count only.

No redirect revalidation loop exists because redirects are categorically denied. This is stricter than Feed fetching and is appropriate for credential-bearing POST requests.

## Provider client and error contract

`ProviderTransport` is an async trait so deterministic client tests can inject a fake. `HttpsProviderTransport` is the production implementation.

```rust
#[async_trait::async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn execute(
        &self,
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError>;
}

pub struct ProviderClient<T> {
    transport: T,
}

impl<T: ProviderTransport> ProviderClient<T> {
    pub async fn generate(
        &self,
        binding: &ProviderBinding,
        request: &StructuredGenerationRequest,
    ) -> Result<StructuredGenerationResponse, ProviderCallError>;
}
```

`ProviderClient::generate` rejects a request whose model differs from the binding model or whose output-token request exceeds the stored policy. It then clones the secret only into adapter encoding, executes the bounded request once, and decodes through the existing `ProviderKind::decode_response` path.

Stable transport kinds:

- `Configuration`
- `InvalidEndpoint`
- `Dns`
- `AddressCount`
- `AddressDenied`
- `InvalidHeaders`
- `Network`
- `TimeoutDns`
- `TimeoutConnect`
- `TimeoutFirstByte`
- `TimeoutBodyIdle`
- `TimeoutTotal`
- `PeerMismatch`
- `RedirectDenied`
- `ResponseHeaders`
- `ResponseTooLarge`
- `Decode`

`ProviderCallError` normalizes adapter and transport errors into the existing AI semantics and carries only provider ID, provider kind, stable call kind, and optional `retry_after_at`. It contains no body or endpoint. A timeout/network error may retain a URL-stripped reqwest source internally, but custom `Debug` hides it.

Retry metadata is advisory only in this slice. Job attempt limits, exponential backoff, cost reservation, rate limiting, and concurrency enforcement remain owned by the future content orchestrator/worker; the provider core persists the policy and exposes typed facts without inventing a second scheduler.

## Testing strategy

### Configuration and secret tests

- Environment list overrides TOML as one unit.
- Empty entries, duplicate IDs, duplicate key bytes, padded/invalid base64, wrong key length, invalid ID, and secret-bearing parse input fail without echo.
- Encryption/decryption round-trip; random nonce changes ciphertext; old key decrypts after active-key rotation.
- Unknown key, wrong provider ID, wrong kind, tampered nonce/ciphertext/tag, oversized/empty secret, and malformed envelope fail closed.
- `Debug`, `Display`, and error chains contain none of the key, plaintext, ciphertext, provider endpoint, or sentinel values.

### Three-database storage contract

- Migration is idempotent and supports up/down/up.
- Required columns/indexes/FK exist on SQLite, PostgreSQL, and MySQL.
- Instance and user providers create/read/list correctly; another user cannot enumerate or load them.
- User deletion cascades user-owned providers but preserves instance providers.
- Stored `encrypted_secret` is not plaintext and decrypts only through the repository binding.
- Kind cannot be changed through update, revision conflicts are detected, disabled providers do not load, and corrupt kind/policy/ciphertext fail closed.
- Boundary values for names, model, capabilities, quota, costs, credential, endpoint, and revisions are exact.

### Transport and client tests

- HTTPS-only endpoint normalization, defaults, path-prefix join, path traversal, credential/query/fragment, literal private IP, and redacted formatting.
- DNS empty/overflow/mixed-private/private/duplicate sets; IPv4, IPv6, and standard NAT64 classification.
- Resolver timeout, connect timeout, first-byte timeout, body-idle timeout, total timeout, missing/mismatched peer, and no second executor call.
- Redirects do not read a body or forward secrets.
- Forbidden/invalid request headers fail before execution.
- Non-2xx bodies are not polled; valid/invalid/multiple `Retry-After` behavior is exact.
- Identity/gzip/Brotli/deflate success and compressed/decoded/ratio overflow failure.
- Client rejects model/policy mismatch, sends the existing four adapter fixtures through a fake transport, maps provider errors, and keeps all sensitive sentinels out of formatting.

## Boundaries

- Always: use the existing canonical adapters; validate at configuration/database/network boundaries; keep transactions short; treat provider/model output as hostile; use exact scope and optimistic revision; run the three-database contract.
- Internally review: dependency addition, schema, key format, endpoint semantics, timeout/size limits, and public error/types before implementation because the user delegated confirmation.
- Never: derive provider encryption from the session secret; store plaintext; log entities/request/response bodies; accept HTTP/private endpoints; follow redirects; use environment proxy settings; expose secret/ciphertext through metadata; add API/UI/job/plugin/MCP claims in this slice.

## Success criteria

- One migration/entity/repository contract passes on SQLite, PostgreSQL, and MySQL.
- Provider kind is selected on create and cannot be changed by the update contract.
- Credentials are AES-256-GCM encrypted with versioned key ID/nonce/ciphertext, AAD-bound to provider ID and kind, and rotation-compatible.
- Provider metadata never contains a credential; only `load_enabled_binding` returns a secret-bearing internal binding after exact scope checks.
- Endpoint normalization and the production transport enforce HTTPS, public-only DNS/IPs, connected-peer pinning, no redirects/proxy, exact staged timeouts, bounded streaming/decompression, and retry metadata.
- `ProviderClient` performs a real non-streaming network execution path behind the existing four adapters while preventing caller-selected model drift.
- Deterministic tests contain no real credential or external provider request, full Rust gates pass, and CI verifies the provider storage contract on all three databases.
- `tasks/todo.md` AI/plugin/MCP items remain unchecked because no content job, official plugin, MCP capability, API, or Reader artifact exists yet.

## Internal self-review

- DDIA: `ai_providers` is the authoritative record; request/response streams are transient; canonical columns avoid provider-specific persistence; optimistic revision prevents lost updates; schema growth is additive.
- API/interface: callers use scope, metadata, binding, transport, and client contracts; raw URLs, ORM models, provider DTOs, and secrets do not cross the boundary.
- Security: dedicated rotatable keyring, AEAD AAD binding, explicit SSRF/DNS/peer controls, redirect denial, bounded decompression, untrusted response parsing, and redacted errors are specified with abuse tests.
- Scope: this slice is a real provider core but not a user-visible AI/plugin feature. No empty plugin/MCP/job abstraction is introduced.
- Open questions: none. Cost/rate/concurrency enforcement belongs to the later job worker and does not block safe provider persistence and transport.
