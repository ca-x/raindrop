# Content Worker Composition v1 Design

## 1. Objective

Compose the existing durable content-job repository, provider broker, hardened Wasmtime host, and real `raindrop.ai-content` component into one recoverable execution path that can claim a summary or translation job, keep its lease alive, execute outside every database transaction, and atomically commit either an immutable artifact or a bounded failure.

This slice proves the complete claim-to-artifact path with the real component and three-backend-safe repository contract. It does not yet claim HTTP enqueue APIs, lifecycle dispatch, MCP transport, Reader sidecar UI, or production-signed component embedding in the release binary.

## 2. Delegated decisions

1. The user delegated internal review and prohibited sub-agent development. The main Agent performs one bounded DDIA/contract/security review.
2. Existing `content_jobs`, `content_job_attempts`, `content_artifacts`, and `content_job_results` remain the record system. No new table or distributed transaction is introduced.
3. Entry, plugin config, provider, and installation rows are re-read immediately before execution. The job's hashes and provider/component snapshot are authoritative; drift fails the attempt instead of silently running new semantics.
4. Network and Wasm execution remain transaction-free. The only success write is the existing fenced artifact/result/attempt/job transaction.
5. Delivery is at least once. Provider request idempotency, claim fencing, and artifact identity make repeated work converge without claiming exactly once.
6. Instance execution concurrency is eight lanes. The database continues to enforce at most two effective RUNNING jobs per user.
7. MCP-enabled configurations are rejected with a stable unavailable code in this slice. The component contract remains MCP-capable; the later MCP client slice supplies authorized tool bindings and transport.
8. Production bundle embedding/signing and `main` startup wiring remain a separate slice because the current repository deliberately contains no production private signing material or committed signed component artifact.

## 3. Scope

### Included

- Complete claim data required to reconstruct the WIT operation request.
- A database-authoritative remaining-attempt duration returned by heartbeat.
- User-visible entry execution snapshot loading and stored-content validation.
- Canonical invocation input hashing shared by future enqueue callers and the worker.
- Provider kind/model/revision revalidation inside `ProviderAiBroker`, closing preflight-to-call drift.
- Detailed plugin execution outcomes containing bounded usage and a host-owned retry hint.
- Official AI processor composition for direct summary and translation with MCP disabled.
- Lease heartbeat, cancellation on lease loss, terminal commit, eight-lane polling, wake-up, and bounded shutdown.
- Real component integration tests that create a job, execute it through Wasmtime and the provider broker, and read the committed artifact.

### Excluded

- New public HTTP routes, provider/plugin administration UI, and Reader artifact UI.
- Lifecycle outbox delivery and automatic intent enqueue.
- External MCP connection repository, tool inventory, broker, audit, or transport.
- Production-signed component generation, embedding, discovery, or startup synchronization.
- Artifact retention, regeneration policy, and historical config storage.

## 4. Module structure

```text
src/content/worker/
  mod.rs          public worker contracts and exports
  input.rs        validated entry snapshot, canonical input hash, WIT request mapping
  processor.rs    official plugin/provider/config composition and failure mapping
  runtime.rs      claim lanes, heartbeat, terminalization, wake-up, shutdown
  error.rs        redacted worker/runtime errors

src/content/jobs/{model,repository,sql}.rs
  claim context, execution entry read, DB-derived attempt budget

src/plugins/runtime/{capability,error,execute,mod}.rs
  provider snapshot context, usage/failure hint, detailed execute outcome
```

Files stay split by responsibility; the worker does not become a provider adapter, plugin registry, or persistence shortcut.

## 5. DDIA record-system and transaction review

The database remains the only recovery source. In-memory lanes and notifications are liveness hints only.

```text
claim transaction
  -> short snapshot reads
  -> fresh heartbeat / DB-derived remaining duration
  -> Wasm + provider calls outside transactions
  -> stop heartbeat
  -> fenced complete_success or complete_failure transaction
```

Safety invariants:

- Database time decides claim validity, lease expiry, attempt deadline, retry due time, and terminal authority.
- A worker never holds a row lock while invoking Wasm, provider HTTP, or future MCP transport.
- Heartbeat failure cancels the in-flight future and performs no terminal write; a later claimer recovers the expired attempt.
- Terminalization begins only after the heartbeat task has stopped, avoiding heartbeat/terminal races.
- Every terminal write rechecks job, user, attempt number, owner, fencing token, live lease, and attempt deadline.
- A success transaction inserts or reuses one immutable artifact, links the job result, finishes the attempt, and finishes the job atomically.

This is at-least-once execution with idempotent effects, not end-to-end exactly once.

## 6. Claim and execution snapshot

`ContentJobClaim` additionally carries the already-persisted, validated values needed by the worker:

- operation and trigger;
- raw idempotency key;
- call-chain ID and remaining depth;
- existing artifact identity and lease fields.

The claim remains constructible only by `ContentRepository`.

`ContentRepository::load_execution_entry(&claim)` returns a bounded `ContentExecutionEntry` only when:

- the user is still active;
- the user still subscribes to the entry's feed;
- the entry exists;
- `entries.content_hash` equals the claim identity snapshot;
- the stored content envelope decodes and revalidates through `EntryContentDetail`;
- rendered text is at most 512 KiB and title/canonical URL obey the WIT request limits.

Missing, invisible, changed, or corrupt entry state never leaks source data through error formatting.

`heartbeat` returns `LeaseDeadline::remaining_attempt()`, computed as `attempt_deadline_at - database_now` inside the same transaction. The worker converts that duration into a local monotonic deadline and a synthetic Unix deadline hint. Process wall time never grants commit authority.

## 7. Canonical invocation input

The frozen input document is canonical JSON:

```json
{
  "entry": {
    "canonicalUrl": null,
    "contentHash": "<64 lower hex>",
    "entryId": "<uuid>",
    "feedId": "<uuid>",
    "sourceLocale": null,
    "text": "<rendered sanitized text>",
    "title": "<title or empty>"
  },
  "operation": "SUMMARIZE",
  "schemaVersion": 1,
  "targetLocale": null
}
```

`ContentInvocationInput` canonicalizes this document and derives a framed BLAKE3 lower-hex hash under `raindrop.content-invocation-input.v1`. Future enqueue APIs must use the same builder; the worker recomputes it and requires equality with `ArtifactIdentity::input_hash()`.

MCP-disabled identity uses a canonical provenance contract under `raindrop.content-mcp-provenance.v1` with `{ "mode": "DISABLED", "schemaVersion": 1 }`. This slice accepts only that hash.

## 8. Plugin, config, and provider reauthorization

Before Wasm execution the processor requires:

- installation key/version/ABI/component digest equals the compiled component and claim identity;
- installation state is `ENABLED`;
- user config exists, is enabled, and its canonical hash equals the job config hash;
- the requested operation is enabled and selects the exact job provider binding ID;
- MCP mode for the operation is disabled;
- provider binding is enabled and visible to the user;
- provider kind/model/revision equal the artifact identity snapshot;
- prompt version and artifact schema ID equal the fixed official v1 operation contract.

`BrokerInvocationContext` carries expected provider kind/model/revision. `ProviderAiBroker` checks them after every binding load, so a provider update between processor preflight and ordinal 1/2 cannot execute under a stale artifact identity.

Revocation that occurs after a provider request has begun may allow that already-admitted request to finish, but fencing and the fixed request snapshot still govern commit. New attempts observe the new state.

## 9. Capability budgets and detailed usage

The direct-operation session uses:

- at most two provider requests (the component normally uses one when MCP is disabled);
- zero MCP calls in this slice;
- provider-policy-derived total input budget;
- operation config maximum output tokens;
- at most 250,000 micro-units of estimated cost;
- remaining recursion depth from the job;
- the fresh DB-derived attempt duration.

`CapabilitySession` records host-owned metrics:

- attempted provider request count;
- attempted MCP call count;
- summed reported input/output tokens and completeness flags;
- summed estimated cost;
- final model label;
- the last broker retryability/retry-at/outcome-unknown hint.

`PluginRuntime::execute_detailed` returns either:

```text
PluginExecutionSuccess { artifact, usage }
PluginExecutionFailure { error, usage, failure_hint }
```

The existing `execute` method remains as a compatibility wrapper that discards detailed accounting.

Attempt metadata is fixed canonical JSON containing only usage completeness flags and schema version. It contains no prompt, entry, provider endpoint, model output, credential, or tool data.

## 10. Failure classification

The worker persists fixed uppercase codes only. Representative mapping:

| Source | Code | Retry |
|---|---|---|
| entry/input/config drift | `EXECUTION_SNAPSHOT_STALE` | no |
| installation disabled/mismatch | `PLUGIN_UNAVAILABLE` | no |
| provider revision/kind/model drift | `PROVIDER_BINDING_STALE` | no |
| MCP requested before transport exists | `MCP_UNAVAILABLE` | no |
| provider unavailable | `PROVIDER_UNAVAILABLE` | broker hint |
| provider rate limit | `PROVIDER_RATE_LIMITED` | yes + bounded Retry-After |
| provider timeout | `PROVIDER_TIMEOUT` | yes, outcome unknown |
| provider/plugin output invalid | `PROVIDER_OUTPUT_INVALID` / `PLUGIN_OUTPUT_INVALID` | no |
| runtime host unavailable | `PLUGIN_RUNTIME_UNAVAILABLE` | yes |
| deterministic trap/limit/invalid invocation | fixed `PLUGIN_*` code | no |

Retry-After is converted from the host hint to a duration and remains capped by the repository's one-hour retry ceiling. Arbitrary guest message text is never persisted.

## 11. Worker runtime

`ContentWorker` owns one repository plus one `ContentProcessor` trait object. For each claim it:

1. performs an immediate heartbeat and obtains the DB-derived remaining attempt duration;
2. starts a heartbeat loop with a 10-second cadence;
3. races processor execution against heartbeat loss;
4. stops and joins heartbeat before terminalization;
5. commits success or failure through `ContentRepository`;
6. treats `LeaseLost`/`AlreadyCompleted` as convergence rather than a second terminal effect.

`ContentRuntime` starts exactly eight lane loops. Each owner is a stable visible-ASCII `content:<instance-uuid>:<lane>` value. Empty queues use a one-second poll plus `ContentRuntimeHandle::notify()` wake-up. Shutdown stops new claims, waits up to 30 seconds for lanes, then aborts remaining futures so claims recover by lease expiry.

Per-job failures do not stop the runtime. A lane panic or supervisor invariant failure stops all lanes and returns a redacted runtime-supervision error.

## 12. Testing strategy

1. Claim contract tests freeze the added operation/trigger/idempotency/call-chain/depth values and DB-derived remaining duration.
2. Input tests freeze canonical JSON/hash, rendered sanitized text, exact size boundaries, visibility, content drift, and corrupt envelope rejection.
3. Provider broker tests prove kind/model/revision drift is rejected before transport.
4. Plugin runtime tests prove detailed success/failure accounting, fail-open success ignoring an intermediate MCP hint, and arbitrary guest text confinement.
5. Processor tests use the real component and a recording provider transport for summary and translation, then verify the committed artifact and attempt usage.
6. Runtime tests cover heartbeat extension, cancellation on lease loss, retryable failure scheduling, eight-lane instance ceiling, wake-up, and bounded shutdown.
7. Existing three-database content terminal tests remain the authoritative atomic-commit proof.

## 13. Commands

```bash
cargo fmt --all --check
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --test content_job_claims -- --nocapture
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --test ai_provider_broker -- --nocapture
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --test plugin_runtime_capabilities --test plugin_runtime_sandbox -- --nocapture
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --test content_worker_processor -- --nocapture --test-threads=1
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --test content_worker_runtime -- --nocapture --test-threads=1
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 clippy --locked --workspace --all-targets --all-features -- -D warnings
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --workspace --all-features
git diff --check
```

## 14. Completion criteria

1. A real summary and translation job can be claimed, executed through the official Wasm component and provider broker, and atomically committed as an immutable artifact.
2. No database transaction spans Wasm, provider, or future MCP work.
3. Lease loss cancels execution and prevents stale terminal writes.
4. Entry/config/plugin/provider/component/prompt/schema/MCP snapshot drift cannot silently change execution semantics.
5. Attempt usage and retry classification are bounded, redacted, and persisted through existing terminal APIs.
6. Eight instance lanes and two-per-user database concurrency are both verified.
7. No public API, lifecycle dispatcher, MCP transport, Reader UI, production embedding, or startup-wiring claim is made.

## 15. Bounded internal review conclusion

- Reliability: database leases and fencing remain the safety mechanism; memory tasks only improve liveness.
- Consistency: snapshot drift fails closed, provider calls are idempotent by job/ordinal, and artifact identity converges repeated success.
- Security: secret/provider transport never crosses WIT; untrusted content remains data; arbitrary guest text is discarded.
- Operability: per-attempt usage and stable codes are queryable without retaining prompts or payloads in errors.
- Open questions: none for this slice.
