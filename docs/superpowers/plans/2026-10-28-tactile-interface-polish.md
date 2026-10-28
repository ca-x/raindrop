# Raindrop tactile interface polish

## Direction and boundaries

- Visual thesis: warm paper, precision-machined controls, and quiet floating materials. Borrow Smartisan's tactile light and Apple's restraint, not ornamental skeuomorphism.
- Content plan: retain the source tree, stable entry queue, article, and progressive settings. Improve hierarchy without moving actions or adding a marketing surface.
- Interaction thesis: recognizable loading silhouettes, immediate press feedback, and short, interruptible surface transitions. Keyboard navigation and routine queue updates remain instant.
- CSS strategy: retain ASTRYX components and existing domain CSS; extend the shared theme and motion tokens rather than introduce a second styling or animation library.
- Preserve light/dark/system themes, independent blur/shadow preferences, reading color schemes, localization, safe areas, and existing focus/scroll restoration.

## First make it work

1. Pin bootstrap loading semantics and pointer/keyboard motion modality with failing tests.
2. Add a branded, non-interactive launch silhouette without delaying real requests.
3. Centralize input modality and replace the mount animation's independent library timing with shared CSS tokens.

## Then make it right

1. Give structure, paper, controls, and overlays distinct material roles.
2. Improve article skeletons and surface entrances without animating reading-list changes.
3. Verify interrupted transitions, keyboard actions, reduced motion/transparency, contrast, and disabled material settings.

## Follow practice

1. Run focused tests, full frontend tests, typecheck, and production build.
2. Inspect the actual browser at desktop and narrow mobile sizes, in light and dark mode.
3. Record results here and update `web/DESIGN.md` to match the implementation.
4. Do not commit, push, create branches, remove worktrees, or change system configuration without an explicit request.

## Action log

- Inspected the working tree, app bootstrap, existing reading layout, preference controls, motion helpers, ASTRYX internals, and existing screenshot. Reason: improve the real product without replacing its component or interaction foundations.
- Searched Nowledge threads for `raindrop design appearance Smartisan Apple`; no matches. Reason: check prior cross-tool decisions without applying another project's working context.
- Queried the local UI/UX database. The first product-wide result was an off-topic landing page and was rejected. The refined style query matched Soft UI Evolution; its restrained depth is relevant, but the current system typography and ink-blue palette remain authoritative.
- Checked the requested audit tools; `crapper`, `dryer`, and `mutator` are unavailable. No tool installation or substitute tool-result claims.
- Added behavior-first tests before implementation. Reason: lock accessible loading and input-specific motion behavior before changing visuals.
- Confirmed the initial tests fail for missing loading semantics and modality tracking, then made both pass. Scoped the non-interactive assertion to the loading region, preserving ASTRYX's accessible skip link.
- Ran the existing desktop reader production contract before material changes; it passed. Added a responsive launch silhouette, paragraph-shaped article skeletons, shared CSS mount motion, and document-level input modality without extra React state.
- Extended the existing material tokens, preserving exact application canvas colors and independent blur/shadow preferences. Inspected real desktop screenshots in light and dark mode.
- Added a system accessibility regression and observed it fail on the ASTRYX modal backdrop's hard-coded `blur(2px)`. Added a shared backdrop token so reduced transparency, increased contrast, and the blur switch also govern the scrim.
- Identified positioning-transform collisions in global press feedback. Excluded the reading-dock, source quick-read, and selection actions from generic scaling; they retain their existing positional transitions and material feedback.
- Replaced the dialog's mount keyframe with reversible CSS transitions and centered transform origin. Reason: rapid dismissal/reopening must not replay an unrelated trigger-direction keyframe or gate input.
- Ran all existing appearance and progressive-settings browser tests: 18 passed. Checked the 360px reading and settings screenshots, preserving controls, safe areas, and horizontal containment.
- Added and observed failing regressions for redundant root-attribute writes and a keyboard-opened mobile drawer's 240ms transition. Reused the input boundary to avoid repeated writes; applied the keyboard policy to drawer, sidecar, immersive, and reading-control surfaces, and close keyboard-dismissed native navigation immediately.
- Queried Nowledge memories for `Raindrop Smartisan Apple 设计`. Results were generic Apple guidance and unrelated music-player decisions, so they do not override this repository or the supplied skills. Graph visualization was skipped because this CLI cannot establish the browser's owner/Space identity.
- Ran the broad browser regression, including setup/login and all reader viewport contracts. The mobile touch-target contract exposed auth-form entry translation producing `43.99999237060547px` bounds for a 44px control; an independent live Chrome probe confirmed it returns to exactly 44px once settled. Removed spatial motion from authentication forms and retained opacity, rather than weakening the test or enlarging arbitrary controls.
- Completed the independent read-only review required by `requesting-code-review`. It found one important composition issue: `--shadow-drop: none` invalidates a combined shadow declaration and removes semantic drag-target feedback. Reproduced it with real `dragstart`/`dragover` events after disabling shadows, then changed the decorative token to a transparent zero shadow. Reviewed all other composed shadow consumers as well.
- Checked a settled dark reader in a separate live browser session. Confirmed toolbar/icon ink is `oklch(0.91 0.012 85)` and selected-paper background is `oklch(0.265 0.01947 85.75)`. Made verification screenshots fast-forward finite transitions so they show the intended resting materials rather than an arbitrary frame of a theme change; no delay was added to the application.

## Verification results

- Red-to-green evidence: bootstrap loading semantics, input modality, repeated-key attribute writes, system backdrop preferences, keyboard drawer motion, shadow-free drag feedback, and the mobile touch-target contract.
- `npm --prefix web run typecheck`: passed.
- `npm --prefix web run test:ci`: 76 files, 548 tests passed.
- `npm --prefix web run build` and `cargo build --release --locked`: passed; browser tests use the resulting embedded production assets.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/opt/google/chrome/chrome ./node_modules/.bin/playwright test interface-settings appearance-settings progressive-settings reader-workspace setup-login mobile-foundation` from `web`: 63 passed, 25 device-specific skips, zero failures.
- Re-ran `interface-settings` after making screenshot capture deterministic: 12 passed, 3 device-specific skips. Desktop, 390x844, and 360x800 light/dark launch, reader, and settings captures are in `web/test-results`.
- Inspected real rendered desktop and mobile screenshots, including settled dark toolbar contrast and the final paragraph-shaped launch silhouette. Physical-device feel and Safari were not tested.
- `npm --prefix web run check:release-contracts` and `git diff --check`: passed.
- Existing build warnings remain: large frontend chunks, deprecated `Atomic::fetch_update`, and `proc-macro-error2` future compatibility. No unrelated build/configuration changes.
- Stored the scoped design decision in Nowledge Mem as decision `85f6779f-5e38-4582-936d-34123d622192`; reason: retain the user's style direction and non-obvious hit-target/composed-shadow constraints for later iterations.
- At the end of the first implementation round, no dependencies, backend behavior, branches, worktrees, commits, pushes, or system configuration had changed. Source changes were left in the current working tree for review.

## Release follow-through (2026-10-28)

- The user requested inspection and continuation, then explicitly authorized publishing a new version when ready. The stable release target is `v0.6.3`, a non-breaking interface-polish update.
- Recovered the scoped Nowledge decision and froze the review baseline at `fc6fd579be243fcd42a8ba09c8cef8863d081667` (`v0.6.2`). Inspected all 18 existing changed and untracked files without moving or discarding work.
- Re-read the public manifests, version verifier, release archive workflow, container publication workflow, and the previous published release. Confirmed the remote main branch matches the baseline and `v0.6.2` is the latest published stable version.
- Reviewed loading semantics, modality listeners, positioned-control transforms, composed-shadow consumers, modal scrims, keyboard drawer dismissal, and breakpoint behavior. No additional blocking defect was found in this pass; the independent first-round review and its fixes remain part of the delivery evidence.
- Synchronized the package versions and lockfiles, version-sensitive setup/About tests, and README release notes. No dependency versions or backend logic changed.
- Fresh candidate verification passed: generated reader contracts, frontend type checking, 76 Vitest files / 548 tests, production Web build, 205 Rust library tests, 10 embedded Web tests, the official native bundle test, and the release binary reporting `raindrop 0.6.3`.
- Ran the complete configured Playwright suite against that embedded release binary: 70 passed, 25 device-specific skips, no failures. Re-inspected the generated desktop launch and dark reader captures plus the 360px settings preview; no new visual defect was found.
- Supply-chain checks reported zero npm vulnerabilities, 405 verified registry signatures, and 125 verified attestations. Release delivery contracts, ASTRYX interface checks, `cargo fmt --check`, and `git diff --check` also passed.
- Prepared an isolated two-viewport browser smoke that accepts a downloaded release binary. After publication it must exercise loading semantics, first setup, reading, the displayed About version, material styling, and dialog dismissal from the actual archive, not just the local build.
- Direct local registry probes are environment-limited: GHCR's anonymous token endpoint returned HTTP 401 and Docker Hub's token endpoint timed out. Do not infer a product regression or claim a successful local pull. The existing remote container workflow performs authenticated publication and asserts both Linux architectures and their attestations; its completed jobs and logs remain a required release gate.
- Remote CI and publication are the next authorized actions. After tagging, verify all five downloadable archives and their checksums, then run the isolated archive browser smoke. Record the final remote state in GitHub Actions, the release readback, and the delivery handoff rather than claiming it before it exists.
- The first commit attempt stopped because this machine has no configured Git author. Confirmed the authenticated publisher matches the existing commit author, then reused that public repository attribution through command-scoped Git options only; no global or repository Git identity settings were changed.
- Safari and physical-device feel remain outside the tested environments. The unavailable `crapper`, `dryer`, and `mutator` tools are not claimed as executed.
