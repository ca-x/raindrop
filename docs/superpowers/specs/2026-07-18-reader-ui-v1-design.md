# Raindrop Reader UI v1 设计

## 1. Objective

交付首个真实可用的 ASTRYX Reader 前端纵向切片。已登录用户可以添加 HTTPS RSS 订阅、查看订阅与未读数、切换 All/Unread/Starred/单 Feed、阅读正文、切换已读与收藏、观察刷新状态并手动刷新。

本切片服务重度 RSS 阅读用户，桌面优先效率，移动端保持单任务专注。原文永远是默认内容，AI、插件、MCP、OPML、分类、搜索与批量已读不进入本切片。

## 2. Assumptions

- 这是响应式 Web 应用，不创建独立原生移动客户端。
- 复用现有 cookie session、CSRF、Reader API v1 与 Subscription API v1。
- `docs/openapi/subscription-v1.json` 是 Subscription TypeScript DTO 的唯一来源。
- Reader entry API 暂无 OpenAPI artifact，本切片先提交 `docs/openapi/reader-v1.json` 与真实 router drift gate，再生成 TypeScript DTO，禁止长期手写第二份 wire contract。
- 不新增运行时依赖、状态管理库、CSS 框架或图标库。

## 3. Tech stack and commands

- React 19.2.7, React Router 7.18.1, TypeScript 7.0.2, Vite 8.1.4。
- ASTRYX core/theme-neutral 0.1.6, StyleX 0.19.0 仅作为 ASTRYX peer。
- Vitest, Testing Library, Playwright。

```text
Generate: cd web && npm run generate:reader-types
Typecheck: cd web && npm run typecheck
Unit: cd web && npm run test:ci
Build: cd web && npm run build
E2E: cd web && npm run test:e2e
Browser: agent-browser against the local production server
```

## 4. Visual and interaction direction

Visual thesis: 高密度编辑阅读工作台，warm paper canvas、ink text、rain-blue state rail，低噪声、少卡片、清晰分区。

Content plan: source tree 用于定位，entry queue 用于选择，article plane 用于阅读与状态动作。主界面不放营销 hero。

Interaction thesis: J/K 打开并标记已读，N/P 只移动选择，M 切换已读，S 切换收藏。高频键盘动作不动画；抽屉与面板使用 160-220ms transform/opacity，并尊重 reduced motion。

参考吸收：

- CommaFeed: 稳定 source、未读优先、键盘效率、网络刷新与本地重载分离。
- Reeder: 正文宽度、serif-led article plane、元数据克制。
- Linear: 可见 focus、快速 selection、URL 与工作区状态同步。

## 5. Component policy

- Root: `AppShell`。
- Workspace: 单个 `Layout`，不得嵌套第二个 Layout。
- Sources: `TreeList`，不使用 SideNav 模拟 Feed 树。
- Entries: `List` + `Item`/`ListItem`，不使用 Table。
- Actions: `Toolbar`, `Button`, `IconButton`, `Kbd`, `Tooltip`, `StatusDot`。
- Mobile navigation: `MobileNav` + `MobileNavToggle`。
- Loading/empty/error: `Skeleton`, `Spinner`, `EmptyState`, `Banner`, `Toast`。
- Subscription creation: ASTRYX `Dialog`/`FormLayout`/`TextInput`/`Button`，不创建通用 wrapper。
- Panel resize: ASTRYX `Resizable`/`LayoutPanel`，仅桌面启用。

## 6. Routes and responsive model

Canonical routes:

```text
/reader/unread
/reader/all
/reader/starred
/reader/feed/:feedId
/reader/unread/entry/:entryId
/reader/all/entry/:entryId
/reader/starred/entry/:entryId
/reader/feed/:feedId/entry/:entryId
```

- `>=1100px`: source tree + entry queue + article reader 三栏。
- `720-1099px`: entry queue + article reader 两区，source tree 放入可访问 drawer。
- `<720px`: source/list/detail 单任务路由。浏览器 Back 恢复 source、entry list scroll anchor 与 selection。
- 360x800 与 390x844 无水平滚动，interactive target 至少 44x44px，支持 safe-area。

## 7. Data contracts and generation

`web/scripts/generate-reader-types.mjs` 同时读取 Subscription 与 Reader committed OpenAPI schemas，确定性生成 `subscription.generated.ts` 与 `reader.generated.ts`。生成文件头记录 source path 与禁止手改声明。CI test 重新生成到内存/临时结果并与 committed output 比较。

生成类型至少包含：

```ts
SubscriptionResponse
SubscriptionPageResponse
CreateSubscriptionRequest
CreateSubscriptionResponse
RefreshSubscriptionRequest
RefreshResponse
RefreshState
ApiErrorEnvelope
```

Reader OpenAPI drift gate 必须驱动真实 `build_router`，逐 endpoint/status 校验 schema、cache 与 security contract。生成后的 runtime validator 必须拒绝缺字段、错类型、未知状态与非数组结构。`contentHtml` 仅作为后端已清洗 HTML 渲染，禁止拼接 AI/plugin 内容。

## 8. State architecture

使用 feature-local `useReducer`，不新增全局状态库：

```text
subscriptionsById + subscriptionOrder
entriesById + queueBySourceKey
detailsById
selectedSource + selectedEntryId
requestGenerationByPane
pendingNewEntriesBySource
scrollAnchorByRoute
```

- Tree/list/reader 读取同一 normalized entities，read/star/unread count 不允许三份状态漂移。
- 切换 source 时递增 request generation；迟到 response 不能覆盖更新的 source。
- M/S optimistic update 同时更新 queue/detail/count，失败完整 rollback 并显示 Toast。
- 新条目只增加 non-disruptive indicator，用户主动 merge/reload 前不重排当前 queue。
- Feed network refresh 与 stored-entry reload 是两个独立动作和文案。

## 9. Loading, empty and error states

- 首屏 skeleton 保留三栏几何，避免 layout shift。
- 空订阅显示添加订阅动作。
- Feed 无条目显示“尚无文章”，并保留刷新动作。
- 401 触发现有登录状态切换；403/422/409/429 使用稳定 API message 与字段提示；500 使用通用 Banner，不暴露 request internals。
- `PENDING`, `READY`, `DEGRADED`, `BACKING_OFF`, `ERROR` 显示为 quiet status，不阻塞阅读。

## 10. Testing strategy

- OpenAPI contract: Reader artifact 与真实 router 双向 drift。
- Generator contract: 两份 artifact drift、deterministic output、forbidden manual type copy。
- API unit: valid/invalid response, CSRF mutations, abort and late-response suppression。
- Reducer unit: normalized state, optimistic update/rollback, unread count, new-entry non-reorder。
- Component integration: empty/loading/error, source switching, entry selection, M/S, add/refresh。
- Playwright: desktop 1280x800, medium 900x800, mobile 390x844 and 360x800；deep link、Back、keyboard、no horizontal overflow。
- agent-browser: local production server real visual/interaction verification。

## 11. Boundaries

Always:

- 优先使用 ASTRYX 组件与 tokens。
- feature 文件默认不超过约 250 行；页面、API、schema、state、components 分离。
- 所有 fetch 支持 AbortSignal，所有 mutation 使用 session CSRF token。
- 键盘与 screen reader 状态同步，focus 在 route/panel 切换后可预测恢复。

Never:

- 不手写第二份 Subscription 或 Reader wire type。
- 不引入 Tailwind、CSS Modules、styled-components 或新 UI/state dependency。
- 不自动合并新条目，不让 AI/plugin 阻塞原文，不在前端发起 Feed URL 网络抓取。
- 不修改 `.superpowers/research/` 或 `node_modules/`。

## 12. Success criteria

- 新用户可以添加 `https://www.ithome.com/rss/`，立即看到 PENDING subscription，并在后台完成后浏览 60 条真实文章。
- 桌面、平板、移动三档布局符合定义，deep link 与 Back 可用。
- All/Unread/Starred/单 Feed 列表和正文来自真实 API。
- M/S 持久化、optimistic rollback、count 同步通过测试。
- Subscription/Reader OpenAPI 与 DTO generation drift gate、typecheck、unit、build、Playwright、agent-browser 验证通过。

## 13. Internal review result

无阻塞问题。分类、批量已读、搜索、OPML、AI sidecar、插件与 MCP 保留为后续独立切片。
