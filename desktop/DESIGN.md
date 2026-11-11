# Raindrop Desktop

## Scope and architecture

An independent, local-first Rust NSH application, not a web wrapper. Reuse the
Raindrop core in process for local SQLite storage, feed fetching and jobs. No
separately installed server, Docker, listening port or mandatory sign-in for local
use. Local data lives in the OS application-data directory, not the checkout.

Optional remote connections import subscriptions and authorized AI metadata into
the local workspace. The reviewed first milestone supports explicit import and
explicit URL-only push of new subscriptions. It does not propagate edits, reading
state or deletions. Reliable bidirectional synchronization is capability-gated:
the current server lacks configuration revisions, tombstones and operation receipts.
Never describe legacy CRUD as reliable bidirectional sync. Never export Provider secrets.
The detailed specification and approval status live in
../docs/superpowers/specs/2026-11-11-raindrop-desktop-local-remote-sync.md.

The product goal is equivalent user workflows to Web using native controls and
shared core behavior, not a visual clone. The current scaffold is not feature
complete; full native settings, remote AI execution and reliable bidirectional sync
remain separate milestones. Imported AI configuration is reference metadata, not
locally usable credentials. Do not claim Web feature parity.

Targets: Linux (GTK 4.14+), macOS 14+ (AppKit), Windows 10 1809+.
Mobile, Web and TUI are explicitly out of scope. Keep OS window decorations,
system fonts and input behavior. NSH owns native rendering and focus. The native-feel
skill's T1/T3 principles apply (native shell owns lifecycle; adopt the platform),
but its WebView/Node architecture is deliberately not used: Rust already owns the
business logic, and there is no JavaScript extension runtime to preserve.

## Design system

Quiet master–detail reader: subscription sidebar, article queue, article body.
Resizable panes, stable navigation, separate scroll owners. At narrow widths hide
secondary panes through explicit navigation, never shrink all three into slivers.
Use the platform UI font, 14 pt chrome, 12 pt metadata, 16 pt reading text.
Spacing uses NSH's 4-point scale. Compact buttons have a 32 pt minimum height;
selection uses a rounded filled surface, not an edge bar. Keep page backgrounds
flat; no cards, gradients, decorative motion or web-style pointer cursors.

All color literals belong in theme.json, with light/dark semantic pairs. The
app-owned ui module wraps nsh-base buttons, fields, navigation and menus, styling
hover, pressed, selection, focus and disabled states. Feature pages compose these.
Use virtual lists with stable IDs, reactive row bindings and no per-cell state.
Article text renders through NSH TextView, never executable HTML or remote WebViews.

Appearance and language are device-local preferences: Chinese/English and
System/Light/Dark. Persist them in an independent, versioned SQLite row; update the
reactive UI only after the atomic save succeeds. Unsupported versions are preserved,
not overwritten. Changing either preference must not remount the reader or discard
search, selection or open forms. Translate interface messages and accessibility
labels, never user-authored article or subscription titles. System delegates to
NSH's appearance service rather than polling OS configuration.

## State, security and interaction

The local database owns durable content, remote identity mappings, local import
suppressions and a durable add-only intent journal. An uncertain POST is never
automatically repeated: reconcile by reading the remote list, or let the user cancel.
One store owns visible query, selection, async
state and request generations. Keep remote accounts and their synchronization
queues isolated. Never infer a deletion from a failed or incomplete remote listing.
Network work runs off the UI thread. Ignore stale results after filter, selection
or session changes. Never log or persist passwords, cookies or CSRF tokens. Sign-in
is required for remote access at every launch until an OS credential-store
integration is available. Local use requires no remote credentials.
HTTPS required except literal loopback HTTP for local development; reject credentials
in URLs, fragments and redirects. Send CSRF on mutations, bound response sizes and
timeouts. External article links accept only HTTP(S) and open through the OS.

Keyboard: platform Mod+R reloads stored articles, Mod+F focuses search, J/K move
selection, M toggles read, S toggles star, Escape returns to the queue in narrow
windows. Editing fields must retain ordinary typing behavior. All controls have
accessible names. Visible feedback covers loading, empty, error, expiry and retry.

## Independent releases

Product, binary and asset prefix: raindrop-desktop. Version in desktop/Cargo.toml
and nsh.json. Tags: raindrop-desktop-vMAJOR.MINOR.PATCH, starting at 0.1.0.
Never bump the server version or publish a Docker image for a desktop tag.
Checks: formatting, Clippy, unit/API contracts, real NSH headless flows, Linux
visual inspection and cloud-native builds on each desktop OS. macOS/Windows
visual checks and signing/notarization must be reported separately, not assumed.
