# RSS ingestion security boundary

Raindrop treats feed URLs, DNS answers, HTTP responses, XML/JSON documents, publisher HTML, validators, and stored content envelopes as untrusted input. HTTP callers use the queue-only `FeedCommandService` plus user-scoped DTOs; they do not receive raw HTTP responses or database entities. Only `FeedRuntime` claims work and invokes the network-owning `FeedExecutor`.

## URL, DNS, and network policy

- Production subscriptions accept HTTPS only. HTTP is available only through explicitly constructed test/development policy.
- URLs are normalized, fragments are removed, credentials and unsupported schemes are rejected, HTTPS redirects may not downgrade to HTTP, and every hop is re-normalized.
- DNS returns at most 16 terminal addresses. Valid CNAME chains are metadata only; at least one terminal A/AAAA record is required for a successful family lookup.
- Every terminal address is checked before use. IPv4 special-use space, IPv6 outside permitted global-unicast space, embedded IPv4 special addresses, Teredo endpoints, forbidden NAT64 forms, and local-use NAT64 ranges are denied.
- DNS-approved addresses are pinned into the HTTP client, and the connected peer must match the approved set. NAT64 snapshot changes force a bounded replay rather than silently changing the authorization set.
- Automatic proxies, cookies, native TLS, HTTP/3, and automatic redirects/decompression are disabled. Redirects are handled explicitly and capped at five.
- Time limits are 3 seconds for DNS, 10 seconds to first byte, 20 seconds per hop, and 30 seconds total. Compressed bodies are capped at 2 MiB; decoded bodies are capped at 10 MiB and a 100x expansion ratio.

## Document and content policy

- XML preflight rejects doctypes, external/entity expansion, malformed XML, excessive depth/events/attributes, and oversized values before projection into feedparser-rs.
- Documents are capped at 10 MiB, 5,000 entries, depth 128, 1,000,000 XML events, 256 attributes per element, and bounded content/title/enclosure budgets.
- Publisher HTML is sanitized with a fixed allowlist. Scripts, styles, forms, iframes, SVG, event handlers, publisher classes/data/style attributes, and active remote image attributes are removed.
- Image source URLs survive only as bounded inert metadata. Stored detail content uses a canonical versioned envelope; every detail read decodes it and re-validates sanitizer idempotence, image indexes, URLs, dimensions, and byte budgets before returning trusted HTML.
- List DTOs never load the content envelope. Detail DTOs return only validated sanitized HTML, inert image metadata, and typed enclosure data.

## Multi-user visibility and pagination

- Entry access always joins the authenticated user's subscription and requires `entry.feed_sequence > subscription.start_sequence`. An opaque entry UUID is only a locator.
- Effective read state is `read_override` when present, otherwise the subscription read-through frontier; starred state defaults to false.
- Lists use descending `(sort_at_us, entry_id)` keyset pagination, never offsets. The first page captures `INGEST_GENERATION`; later pages exclude newer generations.
- Cursors are bounded canonical JSON encoded as URL-safe no-pad base64. Their versioned BLAKE3 filter frame binds the user, state, optional feed, order, and snapshot semantics. Unknown fields, padding, whitespace, noncanonical re-encoding, and cross-user/filter reuse are rejected.

## Refresh ownership, scheduling, and redaction

- `FeedCommandService` owns only the repository and strict production URL policy. Subscribe/manual-refresh/unsubscribe commands persist queue state and return without DNS, HTTP, parsing, sanitization, claim, or execution work.
- `FeedRuntime` is the sole owner of stale-run recovery, scheduled enqueue, claiming, heartbeat, and shutdown coordination. It passes only already-authorized claims to the network-owning `FeedExecutor`; the executor cannot claim additional work.
- MySQL locks the Feed row before claim/terminal authorization; all backends authorize leases with their database clock and fencing token.
- Success/partial persistence, 304 completion, and owned failure persist Feed schedule state, terminal run state, lifecycle outbox records, and lease release in the same transaction. Outbox conflicts roll back the whole terminal result.
- 304 validators remain scoped to their exact final URL. Missing response validators are preserved only when the final URL is unchanged; a redirect clears any absent old-scope validator.
- Validators, URL query strings, response bodies, publisher HTML, and underlying error sources are absent from DTOs and diagnostic output. Repository/service errors use stable typed messages and redacted `Debug` implementations.

## Live smoke

The opt-in smoke uses only the fixed feed URL, never article pages or image resources, and counts actual HTTP executions including redirects:

```bash
RAINDROP_LIVE_RSS_SMOKE=1 cargo test --locked --test live_rss_ithome -- --ignored --nocapture
```

The test requires 50..=100 unique entries, validates every stored detail envelope, proves user-scoped list/detail visibility, permits only a 304 or deduplicated 200 on the second refresh, and enforces exactly two feed HTTP executions. Committed diagnostics contain only the date, count, and statuses—not feed content.

Observed on 2026-07-17 UTC: 60 entries, first refresh `SUCCESS`, second refresh `NOT_MODIFIED` with HTTP 304, and exactly 2 executor calls. No Feed content is recorded.

## Accepted audit exceptions

These exceptions are narrow and must be removed or re-reviewed on every SeaORM or sqlx upgrade:

- `RUSTSEC-2023-0071` (`rsa` Marvin Attack): `rsa 0.9.10 -> sqlx-mysql 0.8.6 -> sqlx 0.8.6 -> sea-orm 1.1.20 / sea-orm-migration 1.1.20 -> raindrop`. Raindrop reaches this through sqlx-mysql's client-side RSA public-key encryption path; it does not perform the RSA private-key decryption operation targeted by the advisory.
- `RUSTSEC-2026-0173` (`proc-macro-error2` unmaintained): `proc-macro-error2 2.0.1 -> sea-bae 0.2.1 -> sea-orm-macros 1.1.20 -> sea-orm 1.1.20 -> raindrop / sea-orm-migration`. This is a build-time proc-macro dependency. Replacement remains tracked in `tasks/todo.md`.

The audit command is:

```bash
cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0173
```
