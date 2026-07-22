# Subscription Backup v0.4.0 Implementation Plan

**Goal:** Add durable multi-target S3/WebDAV OPML backup with manual and interval execution, per-target retention, seven-day history, and a dedicated Settings module.

**Architecture:** Store owner-scoped targets separately from a single interval schedule. Snapshot selected targets into durable jobs, claim them with database lease/fencing, export OPML once, fan out to every target, and record isolated target outcomes. Keep the four backup UI concerns in S3/WebDAV/Schedule/History sub-tabs.

## Task 1: Database and domain contracts

- [ ] Add backup migration and entities for targets, schedule, schedule targets, jobs, and job targets.
- [ ] Add validated enums and DTOs for target kinds, public configurations, retention, schedule, jobs, and target results.
- [ ] Add repository tests for multi-target ownership, secret encryption, schedule replacement, manual enqueue, seven-day history, due enqueue idempotency, lease recovery/fencing, and terminal aggregation.
- [ ] Implement the repository until focused tests pass on SQLite and preserve backend-specific SQL contracts for PostgreSQL/MySQL.

## Task 2: S3 and WebDAV transports

- [ ] Select a maintained SigV4 implementation compatible with the current Rust/dependency graph.
- [ ] Implement shared HTTPS endpoint validation and per-operation DNS/IP checks with redirects disabled.
- [ ] Implement S3 put/list/delete/test against the owned prefix.
- [ ] Implement WebDAV collection creation, put, depth-one list, delete, and test with bounded multistatus parsing.
- [ ] Add deterministic transport tests and retention selection tests.

## Task 3: Runtime and API

- [ ] Add `BackupRuntime` and `BackupRuntimeHandle`; compose and supervise it beside Feed and Content runtimes.
- [ ] Notify the runtime after schedule changes and manual enqueue.
- [ ] Export OPML once per claim, execute all target rows, heartbeat/fence writes, aggregate parent state, and clean local history older than seven days.
- [ ] Add authenticated/CSRF/rate-limited `/api/v1/backups` routes with strict wire DTOs and safe error mapping.
- [ ] Add router tests and OpenAPI drift coverage.

## Task 4: Backup Settings module

- [ ] Add generated-style TypeScript validators/types, API client, and `useBackupController`.
- [ ] Add `Backup` to `PreferencesTab` and render S3/WebDAV/Schedule/History sub-tabs.
- [ ] Implement multiple target lists and focused add/edit forms with test, enable, edit, and confirmed delete actions.
- [ ] Implement schedule interval and grouped multi-target selection plus manual enqueue.
- [ ] Implement seven-day task history with expandable per-target results.
- [ ] Add localized Chinese/English copy, icon-assisted navigation, reduced-motion-safe micro-interactions, and responsive styles.
- [ ] Add component/controller tests for multiple S3 and WebDAV targets and partial jobs.

## Task 5: Existing v0.4.0 UX fixes

- [ ] Close subscription management after the final successful add step.
- [ ] Add the selected-subscription mark-all-read entry and reuse the existing confirmation.
- [ ] Fix Settings navigation/content scrolling, add semantic icons, compact font deletion, and About/version.
- [ ] Run focused tests and browser checks for the complete Settings workflow.

## Task 6: Release

- [ ] Update Rust/Web/release assertions and notes to `0.4.0`.
- [ ] Run formatting, clippy, full Rust tests, Web tests, typecheck/build, Playwright, embedded asset gates, and diff checks.
- [ ] Merge the isolated branch to local `main`, commit intended files only, push `main`, create annotated `v0.4.0`, and push the tag.
- [ ] Verify GitHub Release assets/checksums and published multi-architecture image tags.
