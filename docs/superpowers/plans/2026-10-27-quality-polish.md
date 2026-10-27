# Raindrop quality polish implementation plan

> **For agentic workers:** Execute inline with `executing-plans`; the user requested implementation and publication, with no delegation.

**Goal:** Improve the existing UI's alignment and usability, isolate expensive authentication work, reduce retention database round trips, and publish v0.6.2 after verification.

**Architecture:** Keep the existing ASTRYX controls, warm paper surfaces, ink-blue accent and responsive three-column reader. Move password hashing to bounded blocking tasks while preserving authentication semantics. Batch refresh-history deletes inside the existing transaction and recheck outbox eligibility at deletion.

**Tech Stack:** React 19, TypeScript, ASTRYX 0.1.6, Rust 2024 / MSRV 1.94, Tokio, SeaORM, SQLite / PostgreSQL / MySQL.

**Spec:** User request of 2026-10-27; existing reader, preferences, retention and release delivery specs under `docs/superpowers/specs` and `docs/superpowers/plans/2026-07-18-release-delivery-v1.md`.

## Global constraints

- Preserve current theme and article color separation, semantic controls, keyboard interaction and API contracts.
- Use existing theme and motion tokens; honor reduced motion and touch input.
- Keep deletion batches bounded at 100 and preserve active, recent, leased, retried and unknown events.
- No new dependencies or database schema changes unless verification proves they are needed.
- Publish binaries and container images; verify exact revision, checksums and CI state.

## Review focus

- Small screens and large text: active settings section and footer actions remain visible without page overflow.
- Form validation: the invalid field receives focus, keeping entered values available for correction.
- Password tasks: cancellation cannot release CPU concurrency capacity before the blocking task completes.
- Batch deletion: protected outbox records prevent refresh deletion, and repeated cleanup remains idempotent.
- Themes and motion: light/dark and paper/sepia/gray remain legible; keyboard navigation stays immediate.

### Task 1: UI alignment and feedback

- [x] Update `PreferencesDialog.tsx` to reveal the active navigation button and focus invalid profile fields.
- [x] Align reading-size reset with its control in `AppearancePreferencesForm.tsx`.
- [x] Unify panel gutters, section heading/description flow, footer spacing and control feedback in `reader.css`.
- [x] Verify rendered desktop/mobile, themes, keyboard and reduced motion with meaningful component and browser checks.

### Task 2: Bounded password computation

- [x] Add asynchronous password task checks in `src/auth/password.rs`.
- [x] Execute Argon2 in `spawn_blocking` under a global four-slot semaphore; move the permit into the blocking closure.
- [x] Await hashing/verification in `src/auth/users.rs`, preserving typed errors and unknown-user work.
- [x] Run password, login and setup tests, including cancellation and runtime responsiveness.

### Task 3: Retention batching

- [x] Pin batch behavior with mixed protected and eligible orphan notifications in `tests/feed_retention_contracts.rs`.
- [x] Batch both refresh and orphan deletes in `src/feeds/retention.rs` with parameterized identifiers and deletion-time guards.
- [x] Run local retention/migration contracts and inspect query plans; add explicit PostgreSQL/MySQL CI contracts as a required gate before tagging.

### Task 4: Verification and release

- [x] Complete scoped Rust review, full Rust tests, clippy, formatting, generated contracts, typecheck, Vitest, production build and Playwright.
- [x] Bump application/npm version and version assertions to 0.6.2; update release notes.
- [ ] Commit and push, publish v0.6.2, monitor all required workflows and verify distributed binaries/images.

### Task 5: AI reliability (user follow-up)

- [x] Reproduce premature non-streaming model timeouts; use the existing total request deadline while preserving connect, body-idle, response-size and DNS/peer limits.
- [x] Recover task status/result reads after transient network failures and provide an explicit read retry without enqueueing duplicate model work.
- [x] Preserve the open settings dialog and its draft across the compact breakpoint by keeping the reader component tree stable.
- [x] Run local regression verification and include these fixes in the v0.6.2 release candidate; require the full remote CI before tagging.

## Verification evidence before publication

- Rust workspace / all features: 774 passed, 1 ignored; strict Clippy, formatting and rustdoc passed.
- Frontend: 542 Vitest tests, typecheck, generated Reader contracts and release contracts passed.
- Release-mode library: 205 passed; embedded Web: 10 passed; embedded official AI plugin: 1 passed. Production assets and executable were rebuilt after the last UI change.
- AI transport: 23 focused tests passed, including an actual HTTP response delayed beyond ten seconds and deadline/idle/size checks.
- Production browser checks exercised the reader, translation, themes and settings. After correcting test fixture isolation and waiting for a resized layout, the seven new settings/AI recovery cases passed across all configured sizes. The complete CI browser gate is required before tagging.
- SQLite production-delete query plans use refresh ID lookup rather than scanning the pending status queue. PostgreSQL/MySQL retention must pass against real CI services before the release tag is created.
- Publication and downloaded binary/container readback follow after the candidate is committed; those remote results are not implied by local tests.
