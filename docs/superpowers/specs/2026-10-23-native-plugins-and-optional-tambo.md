# Native official plugins and optional generative UI

## Authority and scope

The user explicitly authorized replacing the existing WASM execution layer on
2026-10-23. This supersedes the WIT/Wasmtime-only execution requirements in the
July plugin designs. Rust is preferred; a Go rewrite is not required. Tambo is
an optional enhancement, not a dependency of reading, summary, or translation.

## Runtime

- Cargo builds and links the official Rust content plugin into the executable.
- Rutis manages activation, typed dependencies, service ownership and cancellation.
- The production worker invokes the registered native service, not a second,
  parallel processor. No compiler, WASM files or external runtime is needed.
- Official plugins are trusted native code, not sandboxed code. Do not advertise
  fuel, linear-memory limits or preemptive CPU isolation after this migration.
- Preserve broker authorization, tenant boundaries, deadlines, budgets, request
  validation, output validation, provider secret isolation and durable job fencing.
- Preserve registry installation IDs, user configurations, grants and artifact
  history. Changing the ABI/digest invalidates stale invocation snapshots, not
  user data. Keep existing storage/API field names for compatibility.
- Persist optional interaction settings in the existing tenant-scoped `plugin_kv`
  store (`interaction.tambo.v1`), with schema version and optimistic revisions.
  Rutis is not a persistence layer. Do not duplicate these settings in browser
  storage, overwrite AI operation configurations, or put secrets into plugin KV.
- Keep signed bundled metadata and production Web-bundle verification. The
  native bundle identity hashes build inputs instead of a WASM component.
- Do not enable paid AI operations merely because the plugin is bundled.

## Optional Tambo

- Disabled by default. Self-hosted Tambo is the supported deployment direction;
  Tambo Cloud announces shutdown on 2026-10-31.
- Load its SDK only when the user explicitly opens the optional interaction.
- Never pass Raindrop provider secrets to the browser or to Tambo. Send article
  context only after an explicit user action and disclose the destination.
- Keep normal reading, summarization and translation working when unconfigured,
  unavailable, cancelled or disabled. Register bounded, safe React components;
  never render model HTML or execute model-provided code.
- Resolve the schema converter through its published ESM entry during Vite builds;
  the SDK's CJS entry otherwise sends dynamic vendor imports through the CJS parser.
- Plugin settings are part of the Raindrop database backup, not the existing
  subscription-only OPML export. Tambo conversation history belongs to the
  separately backed-up self-hosted Tambo database.
- Preserve same-origin CSP and authenticated user isolation at the integration
  boundary; do not weaken global security headers for arbitrary remote services.

## Implementation ledger

1. Replace WIT glue with native typed protocol and async host capabilities.
2. Mount the official service through Rutis and wire production execution,
   cancellation and cleanup. Retain business validation and broker tests.
3. Replace nested WASM builds with deterministic signed native-build metadata;
   update CI, distributions and current documentation.
4. Implement opt-in Tambo configuration, safe UI components and interaction,
   with failure handling and focused frontend/backend tests.
5. Run focused tests, full Rust/frontend gates, production embedding checks and
   review the final diff. Release only after verification.

Ruling: historical signed-manifest verification remains available for persisted
metadata and contract fixtures, but cannot load or execute arbitrary plugins.
Ruling: no third-party native loader, Go bridge, Node runtime or plugin marketplace
is introduced by this migration.

## Screenshot review

| Before | After | Why |
| --- | --- | --- |
| Compact sidecar scrolls the composer out of view | Bounded conversation, independently scrolling messages, fixed composer | Keep Send and Stop reachable without hiding the source article |
| Starting removes the focused consent button | Focus stays in the panel, then moves to the composer; Escape restores the toolbar trigger | Preserve keyboard continuity through lazy loading |
| Stream updates require manual scrolling | Follow the latest message unless the reader scrolls back | Keep new output visible without pulling the reader away from earlier text |

## Verification and release gate

- Rust 1.94.0: format check, all-target/all-feature Clippy with warnings denied,
  and the workspace all-feature suite pass (766 tests). Local PostgreSQL/MySQL
  contracts are environment-gated; SQLite migration/reopen is exercised locally.
- Frontend typecheck, generated-contract drift check, delivery-contract check,
  517 Vitest tests, and the production Vite build pass. Local Node is 24.22.1;
  CI retains its pinned Node 26.4.0 configuration.
- Release binary build and 209 release library/embedding tests pass. Chromium
  production E2E passes 38 tests; 22 viewport-conditional cases are skipped.
  The real SDK is exercised against deterministic v1 SSE fixtures, not a live
  self-hosted model deployment.
- A separate production process saves opt-in at revision 0, shuts down, and
  restarts against the same SQLite database without Tambo environment settings;
  the existing session still reads `enabled: true`, `revision: 0`, and
  `configured: false`. Temporary review processes are shut down afterward.
- Production npm dependencies audit clean; 445 registry signatures verify.
  Full npm audit still reports nine pre-existing development dependency findings
  (one critical, six high, two moderate). Their affected versions are unchanged
  from `ab06ef4`; that baseline's CI run `39805296803` also fails specifically
  at Supply-chain audit / Audit known vulnerabilities. Do not disable this gate
  or silently include unrelated dependency upgrades. A new release tag remains
  gated on an authorized resolution.
