# Subscription Backup Design

Date: 2026-07-22
Release: v0.4.0

## 1. Objective

Raindrop can export each user's subscriptions and categories as OPML and deliver the same immutable artifact to one or more independently configured S3-compatible and WebDAV targets. Users can run a backup immediately or enable a recurring schedule, apply retention per target, and inspect the last seven days of task history.

The feature lives in a dedicated top-level Settings tab. S3, WebDAV, Schedule, and History are separate sub-tabs so credentials, orchestration, and operational results are never stacked into one long form.

## 2. Confirmed product decisions

- A user can create multiple S3 targets and multiple WebDAV targets.
- Each target has its own enabled state, display name, connection settings, credentials, and retention policy.
- One schedule selects any non-empty combination of the user's enabled S3 and WebDAV targets.
- A job exports OPML once, then uploads the exact same bytes to every selected target.
- Target attempts are isolated: one failed upload does not prevent other targets from completing.
- A job is `SUCCEEDED` when every target succeeds, `PARTIAL` when at least one target succeeds and one fails, and `FAILED` when no target succeeds.
- Manual jobs and scheduled jobs use the same queue and worker path.
- Remote retention is evaluated independently after each successful target upload. `retainCount` and `retainDays` are optional; when both are set, both limits apply.
- History is queryable for the latest seven days. Local history cleanup and remote retention are separate concerns.
- Backup contents are OPML only and include subscription titles and category organization through the existing `FeedRepository::export_opml(user_id)` contract.
- Backup artifacts are plain, standards-compatible OPML by default and v0.4.0 does not add file-level encryption. HTTPS still protects transport; target credentials remain encrypted at rest and cannot opt out.
- Credentials use the existing `ProviderSecretKeyring`; secrets are encrypted at rest and are never returned after save.
- Historical single-target data does not exist and requires no compatibility layer.

## 3. Out of scope

- Restoring directly from a remote target.
- Backing up article bodies, read state, stars, AI output, user fonts, or application/database files.
- Cron expressions, calendars, or multiple schedules per user. v0.4.0 uses one interval schedule per user.
- Client-side credential storage or direct browser-to-storage uploads.
- Browsing arbitrary remote objects outside Raindrop's owned prefix.

## 4. Data model

### 4.1 `backup_targets`

One row per S3 or WebDAV destination.

| Column | Contract |
| --- | --- |
| `id` | UUID primary key |
| `user_id` | owner UUID, FK users cascade |
| `kind` | enum storage string: `S3` or `WEBDAV` |
| `display_name` | 1..80 Unicode scalar values, no controls |
| `enabled` | whether the target can be selected or executed |
| `public_config_json` | canonical bounded JSON without credentials |
| `secret_config_ciphertext` | keyring envelope bound to target id and kind |
| `retain_count` | nullable integer, 1..1000 |
| `retain_days` | nullable integer, 1..3650 |
| `revision` | monotonic positive integer used to snapshot configuration |
| `created_at`, `updated_at` | database operational timestamps |

Unique constraint: `(user_id, display_name)`. Maximum 32 targets per user.

S3 public configuration contains `endpoint`, `region`, `bucket`, `prefix`, and `pathStyle`. Secret configuration contains `accessKeyId`, `secretAccessKey`, and optional `sessionToken`.

WebDAV public configuration contains `endpoint` and `prefix`. Secret configuration contains `username` and `password`.

### 4.2 `backup_schedules`

One row per user.

| Column | Contract |
| --- | --- |
| `user_id` | PK/FK users cascade |
| `enabled` | schedule admission switch |
| `interval_hours` | 1..720 |
| `next_run_at` | next database-time slot when enabled |
| `revision` | monotonic positive integer |
| `created_at`, `updated_at` | operational timestamps |

### 4.3 `backup_schedule_targets`

Many-to-many selection between the user's schedule and targets. Composite primary key `(user_id, target_id)`. Repository writes verify both rows have the same owner. Disabling a target preserves selection but excludes it from future jobs until re-enabled.

### 4.4 `backup_jobs`

Durable parent task and fencing boundary.

| Column | Contract |
| --- | --- |
| `id` | UUID primary key |
| `user_id` | owner UUID |
| `trigger_kind` | `MANUAL` or `SCHEDULED` |
| `scheduled_for` | nullable schedule slot; unique with user for scheduled idempotency |
| `status` | `QUEUED`, `RUNNING`, `SUCCEEDED`, `PARTIAL`, `FAILED` |
| `target_count` | immutable selected target count |
| `lease_owner`, `lease_token`, `lease_until` | claim/lease/fencing fields |
| `last_error_code` | safe aggregate code only |
| `created_at`, `started_at`, `completed_at` | operational timestamps |

Only `QUEUED` work can be claimed. Expired `RUNNING` work is recovered to `QUEUED` before claim. A claim increments `lease_token`; every heartbeat and terminal write requires `(job_id, owner, lease_token)` and a non-expired lease.

### 4.5 `backup_job_targets`

Immutable target snapshots and per-target outcomes.

| Column | Contract |
| --- | --- |
| `id` | UUID primary key |
| `job_id` | FK backup_jobs cascade |
| `target_id` | nullable FK backup_targets set null |
| `target_kind`, `target_name`, `target_revision` | immutable UI/audit snapshot |
| `object_key` | deterministic Raindrop-owned remote key |
| `status` | `QUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED` |
| `byte_size` | nullable successful artifact size |
| `error_code` | nullable safe code |
| `started_at`, `completed_at` | timestamps |

The job-target row does not copy credentials. Execution loads the current owner-scoped target and requires the snapshotted revision to match; changed or deleted targets fail with `TARGET_CHANGED` rather than uploading with surprising configuration.

## 5. Queue and scheduling semantics

1. The scheduler wakes at startup, notification, and a bounded polling interval.
2. In one transaction it locks a due schedule, snapshots all selected enabled targets, inserts a scheduled job plus job-target rows, advances `next_run_at` from the claimed slot, and commits.
3. Unique `(user_id, scheduled_for)` makes duplicate schedulers harmless.
4. Workers atomically claim one job with a 30-second lease and heartbeat while executing.
5. The worker exports OPML once after claim and keeps it in memory; OPML is bounded by the existing 10 MiB limit.
6. Target rows execute sequentially in v0.4.0 to keep outbound concurrency and memory bounded. Each target receives the same bytes and deterministic key.
7. The worker records each target terminal result under the active fencing token, then derives the parent status.
8. A shutdown stops new claims, lets the current bounded request finish when possible, and otherwise leaves the lease to expire for recovery.

Manual enqueue accepts explicit target ids. Duplicate ids are rejected; ids must be owned and enabled. The UI defaults selection to all enabled targets.

## 6. Remote object ownership and retention

Object keys are generated by Raindrop and cannot be supplied by the browser:

```text
<configured-prefix>/raindrop/<opaque-user-hash>/subscriptions/
  raindrop-subscriptions-<UTC-basic-timestamp>-<job-id>.opml
```

The opaque user hash is a stable BLAKE3-derived identifier and does not reveal the user UUID. Prefix normalization rejects `..`, backslashes, query fragments, control characters, and values longer than 512 bytes.

Retention listing and deletion are restricted to the exact owned `.../subscriptions/` prefix. Objects are eligible only when their filename exactly matches Raindrop's versioned backup pattern. Unknown objects are never deleted.

- `retainCount = N`: after sorting recognized objects newest first, objects after N are eligible.
- `retainDays = N`: recognized objects older than N * 24 hours are eligible.
- If both exist, the union of eligible objects is deleted.
- Retention failure does not change a successful upload into a failed upload; it records `RETENTION_FAILED` as the target result warning/error code in operational logs and leaves the uploaded backup usable. The public v0.4.0 history reports the upload as succeeded.

## 7. Transport and security contract

### Shared outbound policy

- Endpoints must be absolute HTTPS URLs with no userinfo or fragment.
- Endpoint hosts are resolved before every operation. Loopback, link-local, private, documentation, benchmark, multicast, unspecified, and other non-public address ranges are rejected, including IPv4 transition encodings.
- Redirects are disabled. DNS/IP changes are revalidated for every operation.
- Connection, first-byte, total request, and response-size limits are bounded.
- Logs and public errors never include credentials, URL query values, authorization headers, upstream bodies, or complete object URLs.
- API mutations require session authentication, CSRF, user-scoped rate limiting, strict JSON, and ownership checks.

### S3

- Supports S3-compatible HTTPS endpoints, region, bucket, prefix, optional path-style addressing, access key, secret key, and optional session token.
- Requests use AWS Signature Version 4 through a maintained signing implementation; no incomplete hand-written signer is permitted.
- Required operations: put object, list owned prefix, delete recognized owned object, and lightweight connection test.
- Bucket names, region, and object keys are validated and bounded before signing.

### WebDAV

- Uses HTTPS `PUT`, `PROPFIND Depth: 1`, `MKCOL`, and `DELETE` with Basic authentication.
- Collection creation is limited to normalized configured/owned path components.
- `207 Multi-Status` XML parsing is bounded and rejects external entities/DOCTYPE.
- A connection test performs an owned collection probe and never writes outside the target prefix.

### Credential lifecycle

- Creating a target requires all credentials.
- Updating public fields can omit secret fields to retain the stored secret.
- Supplying any secret update requires a complete valid secret set and replaces the encrypted envelope atomically.
- Responses expose only `hasCredentials: true`; plaintext and ciphertext are never serialized.
- Target Debug implementations omit endpoint details and all secret material.

## 8. HTTP API

Base path: `/api/v1/backups`.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/targets` | list all owner-scoped targets |
| `POST` | `/targets` | create S3/WebDAV target |
| `PATCH` | `/targets/:id` | update enabled/public/secret/retention fields |
| `DELETE` | `/targets/:id` | idempotently remove target |
| `POST` | `/targets/:id/test` | validate/decrypt and run bounded connection test |
| `GET` | `/schedule` | get schedule and selected target ids |
| `PUT` | `/schedule` | replace interval/enabled/selected target ids |
| `POST` | `/jobs` | enqueue manual job for explicit target ids |
| `GET` | `/jobs?since=...&limit=...` | latest seven-day history, newest first |
| `GET` | `/jobs/:id` | owner-scoped job with target results |

Mutation success returns `Cache-Control: no-store`. Manual enqueue returns `202` and `Location`. Target create returns `201`. Delete returns `204` for both existing and absent owner-scoped ids.

Stable public error codes include `VALIDATION_ERROR`, `UNAUTHORIZED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`, `RATE_LIMITED`, `BACKUP_KEYRING_UNAVAILABLE`, `TARGET_CHANGED`, `TARGET_UNREACHABLE`, `TARGET_AUTH_FAILED`, `TARGET_PROTOCOL_ERROR`, `BACKUP_EXPORT_FAILED`, and `INTERNAL_ERROR`.

## 9. UI/UX contract

Settings navigation includes an icon-assisted `Backup` top-level tab. Its content has four visible sub-tabs:

- **S3**: compact target cards/list, status, retention summary, add button, edit/test/enable/delete actions.
- **WebDAV**: the same list behavior with WebDAV-specific fields.
- **Schedule**: enable switch, interval-hours control, a grouped multi-select of all targets, next-run summary, and primary `Back up now` action.
- **History**: seven-day tasks with status, trigger, timestamp, duration, and expandable per-target rows.

Adding/editing a target uses a focused form surface rather than expanding every credential inline. Saved secrets render as `Configured`, never masked fake values. Destructive delete requires confirmation. Empty states explain the next action.

Motion follows `$emil-design-eng`: frequent tab and row navigation is immediate; occasional add/edit dialogs use existing dialog motion; press feedback is 100-160 ms; status disclosure uses opacity/transform under 200 ms; no `transition: all`; and `prefers-reduced-motion` removes movement while retaining useful color/opacity feedback.

## 10. Verification

- Repository tests cover multiple targets of both kinds, ownership, encrypted secret non-disclosure, revision mismatch, schedule target selection, due-slot idempotency, claim fencing, parent partial status, seven-day filtering, and retention prefix safety.
- Transport tests use local deterministic fakes and assert redirect rejection, safe error mapping, signing/WebDAV method contracts, and bounded parsing.
- Router tests cover authentication, CSRF, strict JSON, cache headers, mutation rate limits, non-enumerating delete, and safe errors.
- Web tests cover target lists, multiple selection, secret retention on edit, schedule save, immediate enqueue, target-level history, and the four-tab information architecture.
- Full Rust, Web unit/type/build, and browser suites must pass before `v0.4.0` is tagged.
