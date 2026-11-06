# Long translation and summary questions Implementation Plan

> **For agentic workers:** Use executing-plans to implement the tasks in order. The user explicitly requested implementation and release in the current session.

**Goal:** Complete long translations and let readers ask follow-up questions below summaries.

**Architecture:** Share deterministic text budgeting and UTF-8 chunking between the official Rust plugin and reader translation service. Extend the existing signed native runtime with a bounded question method using its current host capability broker. Keep conversation state in the current reader session and reuse ASTRYX controls.

**Tech Stack:** Rust 2024, MSRV 1.94, Axum, Rutis, React, ASTRYX, Vitest, Playwright.

**Spec:** ../specs/2026-11-06-long-translation-and-summary-questions.md

## Global constraints

Use the user-requested `models-dev-rs = "0.1"`; no new database tables. Existing Provider and deployment policy, authentication, ownership, CSRF, official signing, costs and cancellation apply. Complete artifact publication remains atomic. Test credentials never enter source or public notes.

## Review focus

UTF-8 and paragraph boundaries; later-chunk failures; per-call versus invocation budgets; stale account/article requests; input-method composition and reduced motion.

### Task 1: Content budgets and complete long translations

Modify the official plugin operation, host processor/capability bounds and reader translation batching. Add `text_budget::translation_tokens(&str, &str) -> u32` and lossless `split_text(&str, u32) -> Vec<&str>`. Keep configured output ceilings per request and apply content estimates below those ceilings. Split oversized reader segments, assign fragment IDs, validate every response, and reassemble fragments in order.

- [x] Add failing tests for differing short/long budgets, long single paragraphs, UTF-8, more than 4096 total output tokens and failure on a later chunk.
- [x] Implement bounded chunks and aggregate accounting, without returning partial artifacts.
- [x] Run plugin and translation regressions, strict Clippy, workspace tests and live long-article translation.

### Task 2: Article questions through the official runtime

Add a typed question payload and answer schema. The existing runtime validates the ordinary operation request and plugin identity before dispatching the question method. Publish the already configured processor through ContentRuntimeHandle, retaining the same runtime/broker and shutdown. The authenticated article question API obtains the owned entry and summary config on the server and supplies bounded alternating history as untrusted context.

- [x] Test validation, ownership, disabled configuration, malformed output, timeout/cancellation and bounded history.
- [x] Implement the signed plugin method, broker schema and authenticated API.
- [x] Verify a real two-turn question against the supplied service.

### Task 3: Composer, interaction and delivery

Place a session-scoped composer beneath a successful summary. Reuse ASTRYX TextArea/Button/Banner/Text; sanitize Markdown answers with the existing renderer. Retain failed text, allow stop/retry, abort on article/account changes and handle IME-safe submit. Use only subtle opacity feedback for asynchronously completed answers and respect accessibility settings.

- [x] Add interaction regressions, localized copy and API contracts.
- [x] Implement the composer and verify desktop/mobile/reduced-motion in a real browser.
- [ ] Run all final gates; commit/push; await CI; tag and release; verify downloaded packages and image manifests; run real requests from the published Linux binary; clean up task credentials and servers.

## Model capacity and configurable long-text budgets

The later user requirement adds `models-dev-rs = "0.1"` for typed, optional capacity hints. Fetch only the fixed public models.dev catalog, without any Provider credentials. Cache for six hours, back off failed refreshes for ten minutes, keep the last good catalog, and preserve manual setup when unavailable. Prefer exact provider-qualified IDs, then normalize gateway namespaces and separators (dots, hyphens, underscores, colons), preserving digits/sizes/variant words. Return unknown on conflicting catalog aliases and label alias matches in the UI; known smaller service limits survive whitespace and aliases. Provider-discovered smaller limits win. Catalog metadata cannot change routing, permissions or existing saved limits.

Raise the application validation ceiling for per-request output to 1,048,576 tokens. New Provider and translation defaults are 32,768; new summary defaults are 8,192. Existing values remain unchanged. Content-based estimates continue below these configurable ceilings, and complete article output can exceed a single request. The capacity action fills the draft only after the user clicks, reserves input space in the shared context window, and leaves the settings editable.

## Verification and review before release

Rust workspace: 819 passed, zero failures (one opt-in external RSS smoke ignored). Strict all-target/all-feature Clippy, formatting and rustdoc passed. Web: 566 tests in 79 files, typecheck, production build and generated contracts passed. Live 32,195-character / 153-section translation retained every section in both native and reader paths; native used three requests and 19,957 output tokens. Two-turn follow-ups and selection translation returned valid content. Desktop and 390px mobile interactions were exercised, including reduced motion and retry after restart.

Independent review findings were fixed with regression tests: trusted native per-call cap, rejected truncated reader responses, retained failed/stopped questions while drafting the next one, and preserved smaller service limits when model input contains whitespace. Separator/gateway alias matching has explicit ambiguous-candidate, version and variant tests. No findings remain open. Final CI and published-asset checks run against the committed release source.
