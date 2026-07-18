# Reader Efficiency v2 Design

## Objective

Deliver the next CommaFeed-inspired Reader efficiency slice: stable-snapshot bulk mark-read, next/previous unread Feed navigation, and Feed-local content search. Raindrop keeps CommaFeed's useful interaction model while improving its storage behavior: bulk mark-read advances one sparse frontier per Subscription instead of inserting or updating one status row per Entry.

Success means the current visible ingestion snapshot can be marked read without consuming later Entries, Shift+J/K moves through unread Feeds in the rendered source order, Feed search covers title/author/summary/rendered content consistently on SQLite/PostgreSQL/MySQL, and the responsive ASTRYX Reader remains keyboard- and mobile-safe.

## CommaFeed reference and Raindrop improvements

- CommaFeed sends the loaded list timestamp when marking all read. Raindrop uses the existing immutable `snapshotGeneration`, avoiding database-clock and client-clock ambiguity.
- CommaFeed stores per-Entry read status. Raindrop advances `subscriptions.read_through_sequence` and only normalizes already-existing sparse overrides, so write volume is bounded by target Subscriptions rather than matched Entries.
- CommaFeed walks the visible tree for next unread navigation. Raindrop uses the same rendered category/feed order and exact server-projected `unreadCount`, with late request rejection already enforced by queue generations.
- CommaFeed searches source content. Raindrop adds a bounded rendered-text projection so HTML markup, JSON envelope bytes, and backend-specific case folding are not exposed as search semantics.

## Public API contract

### List Entries search extension

`GET /api/v1/entries` adds optional query parameter `search`.

- `search` is valid only when `feedId` is also present and `categoryId` is absent.
- The raw UTF-8 query must be `1..=128` bytes after surrounding whitespace is removed.
- Canonicalization lowercases with Rust Unicode case conversion, collapses whitespace, removes duplicate terms while preserving order, and permits at most eight terms.
- All terms use AND semantics against the Entry search projection.
- The canonical term sequence participates in the cursor filter hash. A cursor created for one search cannot be reused for another query or an unfiltered Feed list.
- `snapshotGeneration` continues to freeze ingestion membership only. Existing Entry content can be refreshed without changing `ingest_generation`; therefore search filtering is read-committed rather than a historical content snapshot. Sort order remains `(sort_at_us DESC, entry_id DESC)`.

### Bulk mark-read endpoint

```text
POST /api/v1/entries/mark-read
```

Request:

```json
{
  "snapshotGeneration": 42,
  "feedId": "00000000-0000-4000-8000-000000000101"
}
```

Fields:

- `snapshotGeneration` is required, integer, and non-negative.
- `feedId` and `categoryId` are optional UUIDs and are mutually exclusive.
- Neither source field means all current-user Subscriptions. `feedId` targets one current-user Subscription. `categoryId` targets current-user Subscriptions currently assigned to that Category.
- Unknown but valid owned-scope identifiers and empty Categories are idempotent no-ops; the endpoint does not reveal another user's sources.
- A snapshot larger than the committed `INGEST_GENERATION` counter is rejected with `422 VALIDATION_ERROR` on `snapshotGeneration`.

Response is `204 No Content`. Authentication, CSRF, origin/host checks, `Cache-Control: no-store`, `Pragma: no-cache`, strict JSON content type, duplicate security-header rejection, and stable error envelopes match the existing Entry state endpoint.

## Bulk mark-read transaction

The repository interface is:

```rust
pub enum MarkReadScope {
    All,
    Feed(String),
    Category(String),
}

pub struct MarkReadResult {
    pub changed_subscriptions: u16,
}

impl FeedRepository {
    pub async fn mark_read_for_user(
        &self,
        user_id: &str,
        scope: MarkReadScope,
        snapshot_generation: i64,
    ) -> Result<MarkReadResult, RepositoryError>;
}
```

The command runs in one database transaction:

1. Validate user/source identifiers and snapshot bounds.
2. Lock the active User row. SQLite performs the established no-op `UPDATE`; PostgreSQL/MySQL use `SELECT ... FOR UPDATE`. This freezes Subscription membership and Category assignment for the command.
3. Lock target Subscription rows in ascending Subscription ID order. SQLite already owns the writer lock; PostgreSQL/MySQL use `FOR UPDATE`.
4. For each locked Subscription, compute the maximum `feed_sequence` satisfying:

   ```text
   entry.feed_id = subscription.feed_id
   entry.feed_sequence > subscription.start_sequence
   entry.ingest_generation <= snapshotGeneration
   ```

5. Let `target_sequence` be that maximum. Set `read_through_sequence` to `max(current_frontier, target_sequence)`.
6. Normalize existing sparse rows at or below `target_sequence`:
   - starred rows keep the star and clear any `read_override`, incrementing their revision;
   - unstarred rows with a non-null `read_override` are deleted as neutral state;
   - no new `entry_states` row is created.
7. Increment `subscriptions.state_revision` and update its database-clock timestamp only when the frontier or a sparse override changed.
8. Commit atomically.

The command is idempotent. Work is O(target Subscriptions) statements and O(existing sparse overrides) writes; it never performs O(matched Entries) state writes. The persistent Subscription quota bounds the widest transaction to 1000 Subscriptions.

New Entries committed after the list snapshot have a larger ingestion generation and a larger per-Feed sequence, so they remain above the advanced frontier. Entry content updates retain their original ingestion generation and remain members of the confirmed snapshot.

## Search projection and schema

Add `entries.search_text`, a non-null text projection with an empty default for upgrade safety. The deterministic projection is rebuilt whenever stored Entry metadata or content changes.

Projection order is title, author, summary, then rendered sanitized content. The builder:

- extracts text from sanitized HTML rather than searching the `rdsc:v1` JSON envelope;
- applies Rust Unicode lowercase conversion;
- collapses all whitespace to single ASCII spaces;
- separates non-empty fields with one space;
- truncates at a UTF-8 boundary to at most 60 KiB.

The 60 KiB cap keeps MySQL on portable `TEXT`, bounds migration and storage amplification, and prioritizes metadata and the beginning of article content. The migration backfills in deterministic 32-row keyset batches and is safe to re-enter after a partial run.

Feed-local substring matching intentionally has no new full-text index. The existing `(feed_id, sort_at_us, id)` Feed-list index first narrows the scan, while each backend uses a literal substring operator rather than wildcard `LIKE`:

- SQLite: `instr(e.search_text, ?) > 0`
- PostgreSQL: `position($n in e.search_text) > 0`
- MySQL: `locate(?, e.search_text) > 0`

This v1 contract favors identical behavior and simple schema evolution over three incompatible FTS implementations. Global and Category search remain outside this slice.

## Reader state and pending snapshot behavior

The normalized Reader state adds:

- `snapshotGenerationBySource`
- `pendingSnapshotGenerationBySource`
- `feedSearchQuery`

A replace response stores its page snapshot and clears pending snapshot state. A discover response parks new IDs plus the newer snapshot without moving the visible queue frontier. `mergePendingEntries` promotes both the pending IDs and pending snapshot. Bulk mark-read always sends the visible queue's snapshot, so discovered-but-unmerged Entries remain unread.

Changing source clears Feed search. Submitting or clearing search starts a new queue request generation; late search responses cannot replace a newer source or query.

## Next and previous unread Feed navigation

The client derives navigation from the same order rendered in `CategoryList`:

1. Categories in `categoryOrder`.
2. Subscriptions within each Category in `subscriptionOrder`.
3. Uncategorized Subscriptions in `subscriptionOrder`.

Only Subscriptions with `unreadCount > 0` are candidates.

- From a Feed, forward/backward search begins after/before that Feed.
- From a Category, forward starts at the first Feed in that Category and backward starts at its last Feed.
- From a smart source, forward starts at the first rendered Feed and backward at the last.
- Reaching the boundary does not wrap; it selects smart `UNREAD`. If `UNREAD` is already selected and no candidate exists, navigation is a no-op.
- Shift+J selects the next unread Feed and Shift+K selects the previous unread Feed. Existing editable-field and modal guards apply.
- Selecting a candidate uses the normal source request generation, so a slower prior response cannot override it.

## ASTRYX UI and responsive behavior

- `QueueToolbar` uses existing ASTRYX `Toolbar`, `Button`, `MoreMenu`, `Kbd`, and `TextInput`; no generic custom control is introduced.
- Feed sources show a compact `TextInput` with search icon, clear action, Enter submission, loading state, and a 128-byte validation message.
- Bulk mark-read is offered in the queue action menu for All, Unread, Category, and unfiltered Feed sources. It is absent for Starred and disabled while Feed search is active or no visible snapshot exists.
- Confirmation uses ASTRYX `AlertDialog` and explicitly states that Entries received after the current list was loaded remain unread.
- Next/previous unread actions are visible in the menu on compact layouts and have Shift+J/K keyboard hints on larger layouts.
- The 360x800 and 390x844 layouts keep one task per screen, 44px touch targets, safe-area padding, no horizontal overflow, and no motion on high-frequency keyboard navigation.
- Reduced motion remains authoritative. This slice adds no decorative animation; only existing state transitions may be reused after the final motion opportunity audit.

## DDIA and API internal review

- Reliability: the snapshot is a committed monotonic generation, not a client timestamp; later Entries cannot be consumed by a stale confirmation.
- Write amplification: frontier updates are per Subscription and sparse normalization touches only exceptional state rows. No fan-out status insertion exists.
- Consistency: one transaction locks User then Subscriptions, matching existing Subscription mutation lock order. Individual Entry state mutations lock only one Subscription and never acquire the User lock afterward, so there is no reverse lock cycle.
- Multi-instance behavior: all correctness comes from database locks and monotonic counters; no process-local lock is required.
- Portability: search projection generation is in Rust and literal substring SQL is isolated by backend. No collation-specific `LOWER`, FTS extension, or wildcard escaping is part of the public contract.
- Storage: the searchable projection is bounded to 60 KiB rather than duplicating the maximum 1 MiB sanitized content envelope.
- Pagination: the cursor binds canonical search terms and ingestion snapshot. Content-refresh membership remains explicitly read-committed because existing Entry updates do not receive a new ingestion generation.
- Schema evolution: backfill is keyset-batched, deterministic, and re-entrant. New persistence writes always populate the projection.
- API safety: mark-read uses strict JSON, existing session/CSRF policy, no cross-user existence disclosure, and no internal frontier/revision fields in OpenAPI.

## Testing strategy

- Search projection unit tests cover HTML extraction, entities, whitespace, Unicode lowercase, field priority, UTF-8 truncation, duplicate term removal, and byte/term bounds.
- Migration contracts cover SQLite plus CI PostgreSQL/MySQL column/backfill/re-entry/rollback behavior.
- Repository/API tests cover All/Feed/Category scopes, cross-tenant no-op behavior, stale snapshot preservation, explicit unread normalization, starred-row preservation, idempotency, future snapshot rejection, CSRF/security headers, strict bodies, and database failures.
- Concurrency tests pause an individual Entry state mutation and bulk mark-read on the same Subscription, then prove serializable final state without duplicate sparse rows.
- Search tests cover title, author, summary, rendered content, multi-term AND, special `%/_` literals, Unicode, feed requirement, cross-user isolation, cursor mismatch, and content update behavior.
- Reducer/controller tests cover snapshot replace/discover/merge, mark-read reload, search request races, search clearing on source change, rendered-order navigation, boundary fallback, and Shift+J/K guards.
- Browser tests cover desktop/medium/mobile controls, confirmation focus, search submit/clear, no horizontal overflow, keyboard navigation, pending snapshot protection, and real IT Home data.

## Boundaries

- Always: generate Reader TypeScript DTOs from committed OpenAPI, bind SQL values, keep SQL backend differences in statement builders, preserve sparse state, run three-database CI contracts, commit and push each independent completed task.
- Main-Agent decision: the bounded search projection column and strict mark-read request are authorized by the user's autonomous DDIA/API/UI review delegation.
- Never: mark Starred search results through frontiers, perform per-Entry bulk status inserts, add global/Category search, change Entry ordering, modify `.superpowers/research/`, modify root `node_modules/`, or use subagents.

## Success criteria

1. `POST /api/v1/entries/mark-read` marks exactly the requested current-user source through the supplied committed snapshot and returns 204.
2. Later-ingested Entries remain unread; repeated requests are idempotent; sparse overrides are normalized without losing stars.
3. Bulk write volume is bounded by target Subscriptions and existing sparse rows, never by all matched Entries.
4. Feed-local search finds title, author, summary, and rendered content with identical canonical term semantics on SQLite/PostgreSQL/MySQL.
5. Search terms are cursor-bound, source-scoped, bounded, and isolated across users.
6. Replace/discover/merge snapshot state ensures pending Entries are excluded until explicitly merged.
7. Shift+J/K and visible actions navigate unread Feeds in rendered order with deterministic boundary behavior.
8. ASTRYX controls work at wide, medium, 390x844, and 360x800 viewports without overflow or keyboard/modal conflicts.
9. Rust, Web, OpenAPI drift, Playwright, local `agent-browser`, real IT Home RSS, CI, commit, and push gates pass.

## Out of scope

- Bulk mark-unread.
- Mark-read limited to Starred or arbitrary search matches.
- Global or Category content search.
- Database-native relevance ranking, stemming, language analyzers, or FTS indexes.
- Saved searches, query history, or search suggestions.
- Infinite pagination changes.
- AI summary/translation search, plugin artifacts, or MCP tools.

## Open questions

None. The main Agent resolved the bounded v2 semantics through the DDIA/API/UI review above, as requested by the user.
