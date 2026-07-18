# Raindrop 总体实施路线

权威设计规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`。

当前详细计划：`docs/superpowers/plans/2026-07-18-ai-provider-core-v1.md`。

当前进展：Refresh Observability v1 已由 CI run `29646491921` 验证，AI Provider Adapters v1 已由 CI run `29648253330` 验证。AI Provider Core v1 已由 CI run `29652502840` 完整验证：三数据库 provider 记录、独立可轮换 AES-GCM keyring、不可变 provider kind、canonical capability/quota/cost policy、DNS-pinned SSRF-safe HTTPS transport，以及统一 `ProviderClient` 均已落地。下一 AI 切片是 provider 管理 API/UI 与 content jobs/artifacts，再进入官方 Wasm 插件、摘要/翻译和 Reader sidecar。在这些能力以及 MCP 真正接通前，AI、插件和 MCP 总任务继续保持未完成。OIDC、OPML、排序/阅读游标和真实 `v*` 发布 smoke 仍保留在后续任务。

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
