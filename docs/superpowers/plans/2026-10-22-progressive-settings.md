# Progressive Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task, locally without subagents.

**Goal:** Deliver guided AI setup, quiet background updates, restrained motion, and bounded API discovery in v0.4.22.

**Architecture:** Reuse existing ASTRYX forms and controllers. Keep provider draft validation shared across the connection step and final save. Normalize adapter joins centrally; isolate bounded discovery helpers for deterministic tests.

**Tech Stack:** React 19, TypeScript, ASTRYX 0.1.6, CSS transitions, Rust, Axum, reqwest.

**Spec:** `docs/superpowers/specs/2026-10-22-progressive-settings-design.md`

## Global Constraints

- Preserve existing API contracts, credentials, saved gateway prefixes, and reading selection.
- Use existing theme tokens, no new production dependencies, no keyboard-triggered motion.
- All added interface copy has Chinese and English translations.

## Review Focus

- Editing old providers must not force a wizard or require re-entering secrets.
- Back/forward navigation and failed requests must preserve drafts and ignore stale discovery results.
- Versioned gateway bases must not duplicate API versions or escape their configured prefix.
- Unknown-length upstream responses must stop at the byte limit, including within a slow stream.
- Small screens, keyboard flow, and reduced motion remain usable.

## Tasks

- [x] Add provider draft/connection-step regression tests and implement progressive setup in `ProviderForm.tsx` and `providerDraft.ts`.
- [x] Add plugin next-step guidance and progressive summary options; cover with component tests.
- [x] Refine refresh/new-entry presentation and occasional pointer-only motion with existing tokens; preserve queue semantics.
- [x] Test and implement version-aware endpoint joins plus bounded discovery transport and actionable errors.
- [x] Update bilingual copy and documentation; verify frontend/Rust/browser behavior.
- [x] Synchronize v0.4.22 versions and review the source diff for release.

## Release Follow-through

After the production browser gate, commit the reviewed changes, create annotated tag `v0.4.22`, push main and the tag atomically, and read back both remote refs and workflow status. The tag and workflow runs are the record of this post-commit operation.

## Verification Boundaries

- Model discovery coverage uses deterministic mocks and synthetic streams; no real provider keys or live generation calls are used.
- The full Rust workspace suite runs in the debug profile, matching CI. Release checks use the library and embedded Web/plugin targets because unrelated integration fixtures require debug-only helpers.
- Optional PostgreSQL/MySQL integration endpoints are unset locally; their real-service checks remain in CI.
