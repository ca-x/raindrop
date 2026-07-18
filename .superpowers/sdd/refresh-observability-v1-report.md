# Refresh Observability v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- Preserved the existing public `Refresh.state` values and added the compatible `pendingState`, `lastSuccessAt`, and bounded `entryIssues` fields.
- Distinguished `QUEUED` from `RUNNING` using authoritative refresh-run timestamps rather than a second state store.
- Reused `feeds.last_success_at` so a failed refresh can still report the last successful completion.
- Exposed only bounded `DUPLICATE_ENTRY` counts for degraded refreshes; titles, URLs, GUIDs, parser messages, and internal errors remain private.
- Added focused TypeScript presentation logic and generated Reader DTOs from the committed OpenAPI contract.
- Added one ASTRYX selected-Feed summary for desktop navigation and compact source drawers, with pending refresh actions disabled.
- Added concise English and Chinese queued, running, ready, partial, cooldown, error, retry, and last-success copy.
- Covered the feature at 1280x800, 900x800, 390x844, and 360x800.

## Delivery commits

- `0fa8bf3 docs: plan refresh observability`
- `5f81281 feat: expose refresh observability`
- `6e5fe17 feat: distinguish refresh activity`
- `5ad984a feat: show refresh health in reader`
- `c34e27e test: verify refresh observability`

The report closeout is committed separately with `[skip ci]` after the successful remote run.

## Data and API contract

- No database table or redundant refresh-history projection was added.
- Subscription list and detail projections join the existing `feeds.last_success_at`; exact-run responses join the Feed by `feed_id`.
- `pendingState` is present only for the authoritative queued-without-start and running-with-start-without-completion shapes.
- `entryIssues` is deterministic, bounded to eight public issues, and currently emits only `DUPLICATE_ENTRY` with a count for degraded duplicate drops.
- OpenAPI schemas declare strict required fields, date-time formats, enum values, numeric bounds, and the issue-array bound. Generated TypeScript remains the only Web wire contract.

## Fresh deterministic verification

- `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo test --locked --all-features`: passed with 452 executed tests and one opt-in live RSS smoke ignored by design.
- `npm run check:reader-types`: passed; generated Reader contracts are current.
- `npm run typecheck`: passed.
- `npm run test:ci`: 42 test files and 236 tests passed.
- `npm run build`: passed with Vite 8.1.4 and 2091 transformed modules. The main minified JavaScript chunk is 573.84 kB before gzip and retains the existing chunk-size advisory.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npx playwright test --grep "Reader refresh observability"`: 4 tests passed.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:e2e`: release library 110 tests passed, embedded-Web 9 tests passed, and Playwright finished with 19 passed and 3 intentionally skipped wide-only cases.

The first complete Playwright run found that the new refresh scenario reused the module-level production server after the production-contract test had completed setup. All four viewports failed at the setup heading for the same reason. The refresh scenario now owns a separate production server and closes it in `finally`; both the four-viewpoint regression test and the complete suite pass with that isolation.

## Real-feed browser evidence

The local production application was exercised with `agent-browser` against `https://www.ithome.com/rss/`:

- The network refresh fetched and parsed 60 entries; the Reader queue rendered the first 50 and the selected article body remained readable.
- The selected Feed showed `Refresh complete` and `Last successful refresh` after the terminal success.
- `Refresh feed` and `Reload stored entries` remained separate actions.
- At 390x844 and 360x800, document and body horizontal overflow were zero and the refresh action measured 44x44 pixels.
- Browser console errors and page errors were empty.

Temporary browser state, application processes, and the SQLite directory were removed after verification.

## Motion and AI-slop audits

- `$find-animation-opportunities` found no additional motion worth adding to refresh state changes. The existing ASTRYX pulse already communicates queued and running activity, while the high-frequency Reader workflow remains restrained and reduced-motion safe.
- `$kill-ai-slop` scanned 122 frontend files. Nine textual matches were inspected and were existing valid semantics or fixture data; the new refresh UI had no confirmed AI-slop issue.

## Remote CI evidence

- GitHub Actions run [`29646491921`](https://github.com/ca-x/raindrop/actions/runs/29646491921) passed for `c34e27e` on 2026-07-18.
- Supply-chain audit, ASTRYX Web, Rust current-stable compatibility, Rust foundation and three-database contracts, Windows durable replacement compilation, non-root container health, and release embedding plus Playwright E2E all completed successfully.
- The committed release/E2E job confirmed both lockfiles remained frozen and verified the release version.

## Existing advisories

- Release test/build preparation reports the existing `src/feeds/repository.rs::validate_counts` dead-code warning; the strict all-target/all-feature Clippy gate passes.
- `proc-macro-error2 v2.0.1` remains the tracked future-incompatibility advisory through the SeaORM dependency chain.
- Vite continues to report the existing main-chunk size advisory.
- GitHub annotates the pinned Docker setup/build actions because they still target the deprecated Node.js 20 action runtime and are currently forced onto Node.js 24; the container job passed, and action upgrades remain separate release-maintenance work.

## Explicitly remaining

- Sorting, reading cursors, registration policy, administrator management, and OIDC.
- AI provider adapters, content jobs/artifacts, summary and translation UI, and prompt-injection defenses.
- The first AI plugin, lifecycle host hooks, SDK/sandbox, and MCP client/server support.
- OPML import/export, PostgreSQL/MySQL CI service coverage, settings portability, backup, and restore.
- A real `v*` release and registry smoke covering archives, checksums, manifests, provenance, and SBOM.
