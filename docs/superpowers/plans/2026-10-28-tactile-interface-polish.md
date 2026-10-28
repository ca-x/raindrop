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

## TLS release blocker (2026-10-28)

- Source commit `8c60c8d` is on main, but no `v0.6.3` tag or release was created after discovering Dependabot alert 34 (`GHSA-2mjx-qc3c-rqvc`). The user explicitly authorized fixing the TLS dependency before publication.
- Confirmed the pinned Apify Rustls 0.23.39 fork still contains the vulnerable handshake alignment logic. The application's TLS clients and `dlx`/`impit` share that fork, so changing only the version number or replacing it with unmodified upstream Rustls is not a sufficient fix.
- The maintained `impit-main` compatibility branch includes upstream's 0.23.45 handshake fix. The separate default branch is 0.24 development code and is not an appropriate drop-in update. Verify compatibility, cryptographic provider behavior, and the complete resolved advisory set before choosing a new immutable revision.
- Added a network-free integration regression using a real Rustls TLS 1.3 client and a valid ephemeral P-256 ServerHello. The malformed case appends plaintext EncryptedExtensions in the same record; the control case keeps ServerHello isolated. First run both against the old dependency, then against the repaired dependency. No certificate-verification bypass or external endpoint is used.
- Observed the intended red result with Rustls 0.23.39: the malformed flight returned `Ok(IoState)` instead of a key-epoch error, while the ordinary ServerHello passed. Repeated the same failure inside a network-isolated Bubblewrap namespace with an empty environment, no host home or credentials, read-only executable/system libraries, 16 MiB scratch tmpfs, 16-task cgroup, 1 GiB memory/address-space ceiling, 50% CPU quota, 15 CPU-second and 30 wall-second limits, 64 open files, and 1 MiB file-size ceiling. A per-UID process-limit attempt was replaced with the isolated cgroup because the desktop already has more than 1,100 threads; it failed before running the fixture and is not product evidence.
- Selected immutable Apify compatibility revision `5ce04170965aff5da23154e79f5ee25c37134da1` (Rustls 0.23.45), which contains upstream PR 3265's alignment fix and the maintainer's subsequent ML-DSA verifier fix. Keep the existing `dlx`/`impit` versions, their fingerprint APIs, and the application's explicit ring provider unchanged.
- Audited the saved original lockfile against RustSec snapshot `40f5be6fb12ed93fc591bf4ab23b7d793b3f2239` (2026-10-28). It reports 18 vulnerability entries in addition to the manually reproduced Git-fork TLS flaw. Apply targeted compatible updates to the sanitizer, HTTP/QUIC, certificate roots, compression, constant-time utilities, and runtime dependencies rather than doing a blanket dependency upgrade. The compression-codecs update is necessary to reach the repaired Zstandard major versions through the existing HTTP client.
- The audit also reports unpatched `RUSTSEC-2023-0071` in SQLx's RSA dependency. Preserve supported MySQL connections rather than removing the driver or suppressing the alert. Review the resolved reverse-dependency graph and the MySQL authentication implementation: a public-key encryption-only consumer does not expose the private-key operations affected by the Marvin timing attack. Record this reachability assessment separately from fixed advisories and retain the raw audit finding.
- The first Git HTTPS fetch failed before dependency resolution. Retried through the already working GitHub SSH-over-443 transport using command-scoped URL rewriting; no global Git, proxy, or authentication settings changed.
- The full RustSec inventory exposed four additional Rustls advisories fixed in 0.23.46 that the registry-only audit does not report for the Git fork. The 0.23.45 candidate is therefore not the final repair: `impit` enables certificate Brotli decompression, making that flaw reachable. Added bounded, network-free compression regressions and observed both fail against the compatibility fork in the isolated sandbox while ordinary compressed data still round-tripped.
- Merged upstream tag `v/0.23.46` (`f04804fa13341a9b1dc02b5d1dd8a746f2c4b7fb`) with compatibility revision `5ce04170965aff5da23154e79f5ee25c37134da1` without conflicts, yielding tree `75a71462dddeca20103275ecceab66c719f34474`. Vendor only the resulting Rustls library sources and unchanged dlx library sources with standalone manifests. This avoids publishing an unrelated fork or depending on a nonexistent remote merge commit. dlx's old exact HTTP/2 pin must be relaxed to the patched compatible range; all translation logic remains unchanged. Preserve source provenance and original licenses, and include notices in native archives and the runtime container.
- The first full release compile was rejected by the local build cache because manifest inputs changed during compilation, not because of a Rust error. Finish dependency and documentation edits before restarting compilation; do not change shared build inputs during verification.
- Repaired candidate verification: 52 targeted Rust integration tests passed, including TLS boundary/compression, provider conflict, feed sanitization, translation, and AI transport. Both new TLS suites also passed in the same isolated resource-limited sandbox used for the red runs. Another 205 library tests, 10 embedded Web tests, and one official native bundle test passed; the resulting executable reports `raindrop 0.6.3`.
- Fresh frontend checks passed: generated reader and release-delivery contracts, type checking, 76 Vitest files / 548 tests, production Web build, and release-note punctuation. Rust 1.94 workspace/all-target/all-feature Clippy passed with warnings denied, and rustfmt passed. The installed actionlint shim lacked a selected version, so ran its already installed 1.7.7 executable directly; the modified release workflow passed without changing global tool settings.
- Fresh raw RustSec audit dropped from 18 registry vulnerability entries to one, RSA Marvin. Confirmed with the reverse graph and SQLx source that only public-key OAEP encryption is reachable, not the affected private-key operations; an audit invocation excluding only that reviewed ID reports zero remaining vulnerabilities. Two unmaintained transitive-package notices remain disclosed in `vendor/README.md`. Git/path Rustls source fixes were reviewed separately rather than inferred from the registry-only audit.
- Independent read-only review rechecked all 117 upstream library/build source files byte for byte, standalone manifest dependency/features, all five Rustls security fixes, the single Rustls resolution, provider/fingerprint behavior, regression fidelity, packaged notices, and the RSA reachability assessment. No release-blocking defect was found. Live third-party DeepL interoperability and Safari/physical-device testing are not claimed.
- The full configured production-browser suite passed against the repaired embedded executable: 70 passed, 25 device-specific skips, no failures. Cargo manifest, lockfile, and embedded Web entry-point checksums stayed unchanged throughout verification. Commit and push this repair, then require the new commit's complete remote CI success before creating `v0.6.3`; published archives, checksums, signed-plugin startup, and container manifests remain post-tag gates.
