# v0.3.5 Settings profile and plugin management redesign

## Objective

Make Settings match user intent: editable identity details, readable functional grouping, and a plugin list that scales beyond the current AI plugin without exposing Provider implementation details at the top level.

## Product contract

- Widen the desktop dialog and split navigation from the active settings panel; collapse safely on narrow viewports.
- Keep username as a read-only login identifier. Add optional nickname and email editing, with the nickname used as the visible reader account label and username as fallback.
- Open Plugins on an extensible plugin list containing AI Provider, AI Assistant, and Translation.
- AI Provider independently owns OpenAI-compatible endpoint, model, credential, and availability management.
- AI Assistant owns summary and synthesis. Translation is removed from its public configuration; keyword query/retrieval remain future capabilities and do not get fake controls in this release.
- Translation independently provides full-article translation and word lookup, selecting OpenAI or DeepLX. OpenAI reuses an enabled AI Provider; DeepLX owns anonymous/API-key authentication, official/custom endpoint selection, masked credential rotation, default target language, enablement, and an in-place connection test.
- OpenAI translation includes six built-in prompt profiles plus bounded custom `{{to}}`/`{{text}}` templates. Article output is inserted per paragraph without replacing the source DOM and supports translation-only, bilingual, hover, and side-by-side modes.
- Use `dlx` v3.0.1 from the requested upstream repository, pinned to commit `03054aebd09b54e9934642cd8f11f212e2513065` because crates.io's same-name package is unrelated.
- Connect the enabled Translation plugin to article translation through its selected engine without exposing credentials or accepting article text from the browser.
- Use direct, frequent navigation without animation; keep only restrained press feedback, exact transition properties, fine-pointer hover gates, and reduced-motion fallbacks.

## API and security contract

- Add `GET/PATCH /api/v2/profile` with a committed OpenAPI contract and generated runtime-validated TypeScript types.
- PATCH is strict, authenticated, CSRF-protected, rate-limited, and scoped to the current user.
- Normalize display names to optional trimmed Unicode text (maximum 80 characters, no controls) and email through the existing strict lowercasing validator.
- Preserve unique email semantics and return stable validation/conflict errors without echoing rejected input.

## Verification

- Profile API, migration, OpenAPI, controller, dialog, plugin navigation, and AI detail tests.
- DeepLX persistence/API/client adapter, connection-test, chunking, article translation, settings UI, and reader integration tests.
- Full typecheck, Vitest, production Web build, Rust format/tests/release build, and responsive Playwright inspection.
- Review the final UI against the Emil Design Engineering before/after table and interaction checklist.

# v0.3.5 Entry-open read state and interaction polish

## Objective

Make pointer-opened articles follow the same read-state contract as J/K navigation, then apply a focused interaction audit to the reading controls before publishing a patch release.

## Root cause

`ReaderRoutes` navigated pointer-selected entries without calling the optimistic entry-state mutation used by the J/K open path, so the detail view loaded while list, count, and server state remained unread.

## Product contract

- Opening an unread entry from the queue marks it read before navigation; opening an already-read entry does not toggle it back to unread.
- N/P remains cursor-only, while M continues to explicitly toggle the selected entry's read state.
- Reading-toolbar controls keep hover and touch disclosure while meeting 44px target, spacing, focus, reduced-motion, and safe-area requirements.

## Verification

- Red/green Reader workspace regression test for pointer-open read state.
- Production Playwright checks for persisted read state, pointer and keyboard navigation, 375px/390px portrait, and compact landscape containment.
- Full TypeScript, Vitest, production Web, release contract, Rust release, and GitHub workflow gates.

# v0.3.4 Reader management and contextual toolbar refinement

## Objective

Clarify the boundary between adding/managing sources and editing the selected feed, then make reading presentation controls available only when the reader asks for them while retaining Raindrop's ASTRYX visual language.

## Confirmed root causes

1. Category rows rely on TreeList's generic line box while the chevron, custom SVG and label do not share an explicit row metric, so category icon/label centerlines can drift.
2. Add subscription, manage feed, manage categories and OPML are split across several icon-only toolbar actions and dialogs, obscuring the relationship between them.
3. Appinn serves AVIF article images; the media proxy rejects AVIF, and article scroll updates rewrite enhanced image DOM back to `loading`, causing broken media, blank frames and layout flashing.
4. EntryQueue renders a fixed month/day string once and has no relative-time clock.
5. The subscription query selects `normalized_url`, but the repository DTO and public API omit it, so management can only display the website URL.
6. Reading preferences only support a built-in serif/sans enum, and size controls occupy the high-frequency article action toolbar instead of a contextual reading tool.

## Product contract

- Keep one add/manage dialog containing `Subscriptions`, `Add category`, and `OPML` tabs, following CommaFeed's information architecture.
- Move selected-feed edit/delete into a separate dialog opened from a contextual toolbar action, and show `Feed URL` separately from `Website`.
- Use a two-step subscription flow: analyze/create the feed, then allow title/category confirmation through the existing update API. Reuse controller state; do not create a parallel subscription cache.
- Move size controls into an occasional bottom reading toolbar inside the article plane. Keep read/star/original/AI actions in the fixed toolbar.
- Put font and article-theme selection in anchored Popovers; reveal the toolbar from the desktop bottom hot zone or a touch trigger.
- Relative times update every minute, use the browser locale, render an absolute `<time datetime>` value, and expose the absolute local timestamp as a tooltip/title.
- Recognize AVIF only when its ISO-BMFF `ftyp` box contains an `avif` or `avis` compatible brand. Keep SVG/HTML and unknown bytes rejected.
- Article images preserve a stable bounded box across loading, loaded and error states; failed images show a quiet placeholder rather than collapsing.

## Custom font API and storage

- WOFF2 only; validate `Content-Type`, filename/display name, the complete WOFF2 container, and a bounded decode before storage.
- Maximum 5 MiB per font and 8 fonts per user. Enforce quota transactionally.
- Store bytes in a new `user_fonts` table with a user FK using cascade deletion. Use MySQL `MEDIUMBLOB` so the 5 MiB contract is consistent across SQLite, PostgreSQL and MySQL.
- Store a nullable `reading_custom_font_id` on preferences. A non-null selection must belong to the current user; built-in `readingFontFamily` remains the fallback and v1 remains unchanged.
- Add authenticated v2 routes for list/upload/file/delete. Upload/delete require CSRF and the existing preference mutation limiter; upload admission and bounded global concurrency happen before request-body buffering.
- Font file responses are owner-only, `font/woff2`, `nosniff`, and `private` cached. No filesystem path or original filename is persisted.
- Duplicate content for the same user returns a stable conflict response. Deleting the selected font clears the preference atomically.

## Emil design review

| Before | After | Why |
| --- | --- | --- |
| Add and selected-feed editing mixed in one dialog | `+` for add/manage, contextual edit action for the selected feed | Each action has one clear responsibility |
| Reading controls always visible | Bottom hot-zone/touch dock with font and theme Popovers | Reading presentation stays reachable without occupying the article chrome |
| Failed image collapses after loading | Stable bounded media frame with an intentional error state | Prevents visual flashing and preserves reading position |
| Generic/high-frequency motion | Floating toolbar entrance only, exact `opacity`/`transform` properties, <= 180 ms custom ease-out | Occasional state change gains spatial continuity without slowing navigation or keyboard work |
| No press feedback on custom controls | `transform: scale(0.97)` on `:active` with reduced-motion fallback | Controls immediately acknowledge pointer input |

## Verification

- Rust unit/integration tests: AVIF sniffing, font quotas/type/magic/ownership/delete-selection, subscription `feedUrl`, migrations.
- OpenAPI drift checks and generated TypeScript guards for subscription and preferences v2/font contracts.
- Frontend tests: unified management tabs and two-step flow, feed/site labels, relative-time boundaries/timer, floating toolbar persistence, font upload/delete/select, stable image states, category row structure.
- Browser verification at wide/tablet/mobile sizes, including `https://www.appinn.com/feed/`, keyboard focus, reduced motion, and centerline/layout stability assertions.
- Release gate: format, lint/typecheck, targeted and full tests, production web build, version 0.3.4 consistency, clean staged diff, push `main`, create/push `v0.3.4`, monitor GitHub workflows and release assets.

## Out of scope

- Parsing arbitrary TTF/OTF metadata, third-party font hosting, shared fonts between users, nested categories, and a new feed-discovery endpoint.
- Copying CommaFeed's Mantine styling or dependency choices.

# Raindrop 总体实施路线

权威设计规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`。

当前详细计划：`docs/superpowers/plans/2026-07-19-ai-reader-api-sidecar-v1.md`。

当前进展：Refresh Observability v1 已由 CI run `29646491921` 验证，AI Provider Adapters v1 已由 CI run `29648253330` 验证，AI Provider Core v1 已由 CI run `29652502840` 验证，Content Jobs / Artifacts Core v1 已由 CI run `29655164960` 验证，Official AI Plugin Contract / Registry Core v1 已由 CI run `29657539274` 验证，Wasmtime Component Host Core v1 已由 CI run `29663971184` 验证，ProviderClient Broker Composition v1 已由 CI run `29668410474` 验证，Official AI Component v1 已由 CI run `29671796776` 验证，Content Worker Composition v1 已由 CI run `29676464993` 验证，Production Bundle / Runtime v1 已由 CI run `29681436183` 验证，设置向导响应式修复已由 CI run `29683201079` 验证。AI Reader API / Sidecar v1 已完成 user-scoped Provider/config API、手动任务与重试、ASTRYX 设置和原文优先 sidecar，正在执行最终 release gate；生命周期 dispatcher、MCP transport 和插件管理 UI 仍是后续工作。

## 依赖顺序

1. Foundation/bootstrap：Rust + ASTRYX Web 构建、配置、SQLite、设置向导、管理员、本地登录、内嵌 UI。
2. RSS core：订阅、Feed 租约调度、抓取/解析/清洗、幂等条目、文章列表、稀疏阅读状态。
3. Multi-user organization：分类、用户设置、注册策略、管理员和跨用户隔离契约。
4. OIDC：provider discovery、PKCE、账号绑定、claim 策略和管理 UI。
5. AI content：Anthropic Messages-compatible、OpenAI Responses、OpenAI Chat Completions-compatible、Google Gemini provider adapters，以及任务、摘要、翻译、artifact、配额和缓存。
6. Plugin ecosystem：WIT SDK、Wasmtime host、内容管线、Feed 生命周期 outbox 和示例插件。
7. MCP：Raindrop MCP client，以及 Streamable HTTP/stdio MCP server。
8. Portability/import：PostgreSQL/MySQL 契约、OPML 导入/导出、设置 JSON、保留和备份。
9. Product UX：响应式三栏、完整 ASTRYX 组件化、主题、多语言、键盘和可访问性。
10. Release：完整 CI、多平台 binary、双架构 Docker、运维文档和发布 smoke。

每个阶段必须在主干上形成可运行、可测试的纵向切片；后续阶段开始前先为该阶段编写单独详细计划，并回查总体规格的完成标准。
