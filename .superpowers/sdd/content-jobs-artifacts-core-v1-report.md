# Content Jobs / Artifacts Core v1 Verification Report

Date: 2026-07-19
Branch: `feature/foundation-bootstrap`
Status: LOCAL VERIFIED; CI PENDING

## Delivered scope

- Portable `content_jobs`, `content_job_attempts`, `content_artifacts`, and `content_job_results` schema/entity contracts for SQLite, PostgreSQL, and MySQL.
- User-scoped idempotent enqueue with active-user and subscription visibility checks, entry content-hash snapshot validation, case-sensitive opaque key hashing, semantic conflict detection, and collision fail-closed behavior.
- Exact artifact identity over content/config/plugin/component/provider/model/revision/prompt/schema/MCP provenance, with immutable artifact reuse and no writes to RSS entry source fields.
- Database-time due selection, User-first lock order, per-user active concurrency 2, 30-second lease, monotonic fencing token, and 120/180-second non-renewable attempt deadline.
- Crash recovery that marks expired attempts `ABANDONED`, allocates the next attempt/fence, and terminalizes after the third expired attempt without creating attempt 4.
- Retryable failure recording with 5-second then 30-second default backoff, bounded `Retry-After` up to one hour, unknown-outcome audit, and permanent/exhausted terminal states.
- One atomic success transaction covering artifact insert/reuse, job-result link, attempt success, and job success; injected result/job failures roll back the entire transaction.
- Tenant-scoped job, attempt, result, and exact-identity artifact reads with uniform NotFound behavior across tenants.
- Explicit CI steps that run the same content backend contract against SQLite, PostgreSQL, and MySQL service databases.

This slice is the durable storage/domain core only. It does not execute provider calls, Wasm, MCP, lifecycle fan-out, HTTP routes, Reader UI, or summary/translation prompts.

## Schema and state-machine decisions

- The database is the authoritative record system for job, attempt, result link, immutable artifact, metrics, and safe provenance. In-memory worker state is only ephemeral coordination.
- `content_job_results` removes the jobs/artifacts circular foreign-key problem and makes a reused artifact an explicit persisted relationship.
- Physical idempotency uniqueness is `(user_id, idempotency_key_hash)` so MySQL collation cannot collapse case-distinct opaque keys; the raw key and request hash are compared under the user lock.
- Artifact current/stale is relative to a caller's complete expected identity. No mutable `is_current`/`is_stale` bit is stored.
- Job states are `QUEUED`, `RUNNING`, `RETRY_WAIT`, `SUCCEEDED`, and `FAILED`; attempt states are `RUNNING`, `SUCCEEDED`, `FAILED`, and `ABANDONED`.
- Completion authority requires user/job/attempt/owner/fence plus database-evaluated live lease and deadline. Heartbeat and terminal writes use conditional SQL so a boundary crossing cannot revive an expired worker.
- Lock order is User → Job → Attempt → Artifact → JobResult. SQLite obtains writer serialization with no-op updates; PostgreSQL/MySQL use `FOR UPDATE`.
- Artifact payload is limited to 512 KiB canonical JSON; attempt metadata and artifact provenance are each limited to 32 KiB canonical redacted JSON.

## Security, tenant, and secret review

- Public repository methods always carry a user ID or a typed claim containing the original user; cross-user IDs and missing IDs converge on NotFound.
- Enqueue verifies entry visibility through the user's subscription to the entry feed and does not treat administrator status as content access.
- Provider credential, endpoint credential, headers, system prompt, complete entry text, raw provider response, MCP credential/result, stack, and driver error body are not accepted by the persisted model.
- Stable error codes are bounded uppercase identifiers; metadata/provenance are canonical bounded JSON and are not echoed by repository errors.
- Provider binding ID/kind/model/revision are provenance snapshots, but provider secret storage remains exclusively in Provider Core.
- Content success never updates `entries.title`, `entries.summary`, `entries.sanitized_content`, or `entries.content_hash`.
- Hash lookups always compare the complete raw semantic identity; a digest match is not treated as proof of equality.

## Focused deterministic verification

| Contract | Fresh evidence |
| --- | --- |
| Migration/entities | 1 SQLite migration/index/FK/entity round-trip test |
| Hash/model | 6 integration tests plus 2 framed-hash/domain-separation unit tests |
| Enqueue | 7 tests covering visibility, drift, exact replay, conflict, eight-way concurrency, reuse, and collision |
| Claim/recovery | 8 tests covering due order, single winner, per-user concurrency, heartbeat, stale fence, recovery, and exhaustion |
| Terminal/artifact | 9 tests covering permanent/retry failure, Retry-After, unknown outcome, success, reuse, tenant reads, stale fence, and two rollback triggers |
| Backend entry points | 3 tests; SQLite executed locally, PostgreSQL/MySQL variants are environment-gated and now have explicit CI service steps |
| Feed claim regression | 15 tests passed |
| Provider/feed terminal regressions | 7 provider storage, 5 lifecycle outbox, and 2 feed terminal backend tests passed |

## Commits

- `35d2618 docs: design content job core`
- `0b2c4b6 feat: persist content job records`
- `09ff736 feat: define content job contracts`
- `15c36c8 feat: enqueue content jobs idempotently`
- `3f9cafa feat: recover leased content jobs`
- `5d48bb8 feat: commit immutable content artifacts`

## Final local gates

| Command/check | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --locked --test content_job_migration` | PASS — 1 test |
| `cargo test --locked --test content_job_primitives` | PASS — 6 tests |
| content job hash library tests | PASS — 2 tests |
| `cargo test --locked --test content_job_enqueue` | PASS — 7 tests |
| `cargo test --locked --test content_job_claims -- --test-threads=1` | PASS — 8 tests |
| `cargo test --locked --test content_job_terminals -- --test-threads=1` | PASS — 9 tests |
| `cargo test --locked --test content_job_backend_contracts -- --test-threads=1` | PASS — SQLite executed; PostgreSQL/MySQL skipped without local URLs |
| `cargo test --locked --all-features -q` | PASS — 545 tests listed, 544 passed, 0 failed, 1 ignored opt-in live IT之家 RSS smoke |
| `git diff --check` | PASS |

The ignored test is `ithome_feed_securely_ingests_and_deduplicates`, which intentionally requires `RAINDROP_LIVE_RSS_SMOKE=1` and public network access. Local PostgreSQL/MySQL destructive contract URLs were not configured; the committed CI workflow provides both service URLs and runs the new backend contract explicitly.

## Known exclusions and next boundary

- Provider administration API/UI and credentialed contract probes.
- Worker composition from valid `ContentJobClaim` through provider quota/cost reservation, `ProviderClient`, and `complete_success` / `complete_failure`.
- Official signed `raindrop.ai-content` Wasm Component, versioned WIT/manifest, capability host, prompt/artifact schemas, and summary/translation behavior.
- MCP client broker, MCP audit/tool limits, and Raindrop MCP server.
- Lifecycle delivery fan-out and automatic Feed rule evaluation.
- Reader sidecar, execution API/OpenAPI, artifact current/stale composition, and responsive ASTRYX UI.

All future AI/plugin/MCP todo items remain unchecked. No native processor or route handler is authorized to call a provider directly.

## CI evidence

Pending the pushed documentation/workflow commit. Record the run ID, URL, and seven-job result after the single bounded CI watch.
