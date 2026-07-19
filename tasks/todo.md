# Raindrop 任务清单

## 1. Foundation/bootstrap

- [x] Rust 2024/Axum/SeaORM 工程与 ASTRYX 0.1.6 React/Vite 工程可构建。
- [x] Web 生产资源嵌入 Rust binary，并提供 SPA fallback 和安全响应头。
- [x] 配置实现 defaults < TOML < `RAINDROP_*`，非法值失败且不泄漏秘密。
- [x] SQLite 默认连接、WAL 设置、基础 users/roles/sessions 迁移完成。
- [x] 一次性 setup token、数据库检查、原子配置写入和管理员创建完成。
- [x] 本地 Argon2id 登录、服务端会话、CSRF 和 logout 完成。
- [x] `zh-CN`/`en` 设置/登录/已初始化页面使用 ASTRYX 组件完成。
- [x] Rust、前端和端到端 foundation 验证通过。

## 2. RSS core

- [x] Feed/subscription/entry/entry_state schema 与三数据库迁移。
- [x] URL 安全、SSRF 防护、条件请求、解析、清洗和幂等插入。
- [x] 将 `https://www.ithome.com/rss/` 作为 opt-in/live smoke，覆盖真实抓取、RSS 2.0 解析、HTML 清洗、`60-item` 级入库、二次刷新幂等无重复，以及列表/正文展示；live smoke 与确定性 CI fixture 分开。
- [x] Subscription API/runtime：用户范围 list/detail/create/manual refresh/unsubscribe、调度租约、退避、stale recovery、scheduled enqueue、刷新事件和 OpenAPI drift gate。
- [x] Feed 保留策略。
- [x] Reader API client 从 committed OpenAPI 生成 TypeScript Subscription/Reader DTO，并由 drift gate 阻止手写第二份 wire contract。
- [x] 订阅树、文章列表、阅读器、已读/未读/收藏与刷新终态同步。
- [x] CommaFeed 效率首片：All/Unread/Starred、J/K 与 N/P、M/S、路由/焦点/滚动恢复和非打扰式新文章合并。
- [x] CommaFeed 后续：批量已读快照、下一未读来源和来源内搜索。
- [x] 列表重载与 Feed 网络抓取分离，pending/ready/delayed/error 状态可见。
- [x] 将 queued/running 拆分显示，并补充 partial duplicate failure 的有界条目级反馈、冷却重试时间和上次成功刷新时间。

## 3. Multi-user organization

- [x] 分类：schema、CRUD、订阅分组、Reader category filter 与四视口验证。
- [ ] 排序、阅读游标和用户设置。
  - [x] 用户设置 v1：theme/locale/density/reading scale 的三数据库存储、严格 API、ASTRYX 设置流、首屏 hint 与四视口验证。
- [ ] 注册策略、管理员用户管理和跨用户隔离测试。

## 4. OIDC

- [ ] Authorization Code + PKCE + state + nonce。
- [ ] external identity、账号绑定、claim/domain/group 策略。
- [ ] OIDC 设置和账号 UI。

## 5. AI content

- [x] Provider Core：Anthropic Messages-compatible、OpenAI Responses、OpenAI Chat Completions-compatible、Google Gemini 四类 adapter，独立加密 keyring、SSRF-safe transport 和统一 `ProviderClient`。
- [x] Content Jobs / Artifacts Core：幂等入队、租约/fencing、崩溃恢复、有界 retry、immutable artifact identity 和三数据库原子终态。
- [ ] 官方 `raindrop.ai-content` Wasm 组件、ProviderClient broker composition、摘要/翻译 prompt/schema 执行、额度预留与 artifact 生成。
  - [x] ProviderClient broker composition：用户作用域 binding、四协议 adapter、并发/RPM/token/cost admission、稳定幂等、官方 schema typed validation 与翻译 locale 合同。
  - [x] 官方 no-WASI 组件：Rust guest、摘要/翻译、两阶段 MCP、FAIL_OPEN/FAIL_CLOSED、lifecycle intents、固定 failure code、确定性 componentize 与真实 Wasmtime 测试。
  - [x] Content worker composition：claim/heartbeat、官方 Wasm + provider broker、usage/retry、artifact 原子终态和八 lane runtime。
  - [x] Production bundle/runtime：release/development 签名、二进制嵌入、installation 同步、真实 provider/Wasm ContentRuntime 和启动/关闭接线。
- [ ] provider 管理 API/UI、content execution API 和重试入口。
- [ ] AI artifact UI 与 prompt injection 安全测试。

## 6. Plugin ecosystem

- [x] Contract Core：`raindrop:content-plugin@1.0.0` WIT、官方 manifest/config/artifact schema、五类 lifecycle fixture 和 CI parser gate。
- [x] Registry Core：官方 bundle SHA-256/Ed25519 验证、installation/config/capability grant/KV 三数据库记录、CAS 和配额合同。
- [x] Wasmtime Component Host：无 ambient WASI、fuel/memory/epoch/output 限制、guest bindings 和 host capability broker。
- [x] Official invocation contract repair：完整 MCP tool descriptor、schema digest、config-bearing lifecycle request、逐字段漂移拒绝和 lifecycle capability suspension。
- [x] Official AI component：no-WASI Rust guest、动态 tool-plan 授权、摘要/翻译/MCP/lifecycle 执行和 CI 真实组件门禁。
- [ ] `before/fetched/entry.process/persisted/completed` runtime dispatcher、delivery、outbox 重投与熔断。
- [ ] 插件管理 API/UI、官方组件打包发现、后续 SDK 和第三方 additive 扩展文档。

## 7. MCP

- [ ] 外部 MCP Streamable HTTP/受限 stdio client 和 tool allowlist。
- [ ] Raindrop `/mcp` 与 `raindrop mcp --stdio` server。
- [ ] scopes、token、用户隔离和协议契约测试。

## 8. Portability/import

- [ ] PostgreSQL/MySQL 仓储契约和 CI service tests。
- [ ] OPML 导入预览/提交与 OPML 导出。
- [ ] 设置 JSON 导入/导出、保留、备份和恢复演练。

## 9. Product UX

- [x] ASTRYX `AppShell + Layout + TreeList + List/Item` 响应式三栏。
- [x] `>=1100px` 三栏、`720–1099px` 两区、`<720px` 单任务深链接路由，并恢复列表/正文滚动锚点与返回焦点。
- [x] Reader 规范化实体状态保证树、列表、正文的已读/收藏/计数一致；新条目不自动重排当前队列。
- [ ] 摘要/翻译/plugin artifact 作为非阻塞 sidecar，原文默认且始终可读。
- [ ] light/dark/system、Kami 排版、中文/英文完整覆盖。
- [x] 键盘、screen-reader 语义、reduced-motion 和 390×844/360×800 移动端验证。
- [x] Reader 前端完成后执行 motion 机会审计，并只加入 setup 步骤切换与 pending 新文章提示两处克制动效；`kill-ai-slop` 复扫无确认问题。

## 10. Release

- [ ] 跟踪 SeaORM 依赖链中的 `proc-macro-error2 2.0.1` future-incompatibility；上游修复后升级，current-stable CI 预警 lane 在此之前保持 `continue-on-error`，Rust 1.94 MSRV gate 继续阻塞。
- [x] CI 质量门、依赖审计和 OpenAPI drift。
- [x] Linux/Windows/macOS binary 与 checksums workflow。
- [x] GHCR + 可选 Docker Hub 的 amd64/arm64 Docker workflow。
- [x] 非 root 容器、healthcheck、README、配置和运维文档。
- [ ] 使用真实 `v*` tag 验证 GitHub Release 五个平台归档、`SHA256SUMS`、多架构镜像 manifest、provenance 和 SBOM。
