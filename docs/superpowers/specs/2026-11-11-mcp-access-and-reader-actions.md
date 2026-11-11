# MCP access and contextual reader actions

## Scope

Ship v0.7.0 with a quieter source toolbar and an official, built-in MCP access integration in the existing Plugins settings. This is a React/Astryx application, not an NSH application. Keep its current visual language and layout rather than redesigning the whole reader.

| Before | After | Why |
| --- | --- | --- |
| Current-feed refresh and edit occupy global source-toolbar space | Actions appear together in the current queue’s overflow menu | Place actions next to the object they affect; preserve disabled refresh and dialog focus return |
| Add competes visually with content | Lower resting opacity on precise-pointer devices; full visibility on hover, focus and touch | Reduce visual noise without relying on hover for discovery |
| External AI has no user-controlled reader access | MCP access in Plugins with independent named, expiring, revocable tokens | Explicit consent and account isolation without requiring an AI provider |

## Safety and interface contract

- Expose only `list_subscriptions`, `list_entries` and `get_entry`. No writes, arbitrary HTTP, shell or configuration access.
- Stateless Streamable HTTP at `/mcp`; manual Bearer authentication, not browser-session authentication or OAuth.
- Token CRUD at `/api/v1/mcp/tokens`, scoped to the current login. Mutations require CSRF. Secrets have 256 bits of randomness and are returned only during creation; persist only a digest.
- Bound token names, token count, request bodies, tool pagination and article output. Expiry choices are 30, 90 and 365 days.
- Validate disabled users, expiry, revocation, Origin and transport headers. Treat feed and article output as untrusted data.
- With no valid tokens the integration grants no access; revoking all tokens disables access for that user.
- Existing outbound `host-mcp` content-plugin capabilities are unchanged. This feature does not load arbitrary third-party plugins or weaken the AI trust boundary.

## Acceptance and delivery

1. Verify moved actions, keyboard focus return, refresh disabling and pointer/touch visibility.
2. Exercise token create/list/revoke/expiry, missing or invalid Bearer credentials, CSRF and cross-user denial in Rust integration tests.
3. Exercise UI error recovery, one-time secret lifetime and clipboard fallback with Vitest.
4. Exercise real setup, token management and MCP requests against the embedded production server at desktop and mobile sizes. Capture screenshots only after hiding the token.
5. Run generated-contract checks, TypeScript, Vitest, production web build, Rust tests/format/clippy and Playwright; do not publish on a failing gate.
6. Update version fields, README and MCP documentation; commit and push, verify CI, tag v0.7.0, then verify release archives/checksums and container publishing.
