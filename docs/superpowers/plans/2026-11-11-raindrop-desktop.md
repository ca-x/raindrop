# Raindrop Desktop Implementation Plan

> Execute locally using the executing-plans workflow. An independent reviewer
> audits the requirements and later the implemented security and synchronization.

**Goal:** Native, local-first Raindrop with optional remote subscription sync.

**Architecture:** NSH Rust controls compose a local workspace. The existing Rust
core runs in process without a listener; authorized remote APIs feed a separate,
durable reconciliation layer. Never export remote Provider secrets.

**Tech Stack:** NSH 0.4.0, Rust, existing Axum/SeaORM core, SQLite.

**Spec:** ../specs/2026-11-11-raindrop-desktop-local-remote-sync.md

## Constraints and review focus

- Independent desktop workspace, version and `raindrop-desktop-v*` tags.
- No local Docker and no server listening port for desktop-local operations.
- Native semantics, semantic tokens and app-owned styled controls before screens.
- Never silently merge accounts or Feed URLs, propagate deletes or copy AI keys.
- Recovery must cover remote acknowledgment lost, stale results, incomplete lists,
  offline restart and simultaneous metadata changes.
- Do not claim Web feature parity or unsupported remote capabilities.

## Tasks

- [x] Review the specification and record the supported compatibility sync contract.
- [x] Reuse the local core: private OS data directory, single writer lock, migration,
  local identity/session, runtime startup/shutdown; test persistence and locking.
- [x] Native foundations and reader: reusable controls, navigation, virtual lists,
  article rendering, subscription creation, refresh, search, read/star and keyboard.
- [x] Remote connections: authentication, bounded complete pagination, provider metadata,
  connection isolation, durable nonsecret profiles and explicit capability state.
- [x] Compatibility reconciliation: durable source staging/mappings/suppression,
  explicit imports and URL-only add-new pushes, unknown outcomes never re-POSTed.
- [x] Native connection/sync settings; unavailable full bidirectional mode and
  honest AI metadata/reference states instead of pretending imported credentials exist.
- [x] Device-local Chinese/English and System/Light/Dark preferences; atomic save,
  restart restoration and damaged-sync isolation; preserve active reading state.
- [x] Independent desktop CI/release packaging source contracts; Linux portable ZIP
  builds, archive contents/executable permission checked and extracted app launched.
- [x] Headless/API/recovery tests, Clippy, Linux visual inspection and README screenshots.
- [x] Independent final review: source/visual review approves local + remote
  compatibility development preview after the crash-recovery, pagination-budget
  and background-preference-loading fixes. Not a formal cross-platform release approval.

Verification: 23 desktop tests, 2 root database tests, desktop Clippy, both fmt
checks and desktop actionlint pass. Linux release ZIP is checked and the extracted
application launches; README screenshots are captured from that native build.

## Remaining release and feature gates

- macOS/Windows build, execution, accessibility and native interaction validation.
- Official plugin signing, application signing/notarization and install/update policy.
- Stable server identity, configuration revisions, snapshots/cursors, tombstones and
  transactional operation receipts before reliable bidirectional synchronization.
- Remote article identity/authorization before executing imported remote AI references.
- Full native AI/MCP/backup/OPML settings and local-session renewal/runtime supervision.

Desktop publication uses only the independent `raindrop-desktop-v*` prerelease
lane. Server version and release tags remain unchanged; no local Docker builds
are used. Application signing/notarization and runtime validation on the remaining
platforms are not implied by a successful cross-platform build.
