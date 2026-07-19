# Raindrop 总体实施路线

权威设计规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`。

当前详细计划：`docs/superpowers/plans/2026-07-19-production-ai-bundle-runtime-v1.md`。

当前进展：Refresh Observability v1 已由 CI run `29646491921` 验证，AI Provider Adapters v1 已由 CI run `29648253330` 验证，AI Provider Core v1 已由 CI run `29652502840` 验证，Content Jobs / Artifacts Core v1 已由 CI run `29655164960` 验证，Official AI Plugin Contract / Registry Core v1 已由 CI run `29657539274` 验证，Wasmtime Component Host Core v1 已由 CI run `29663971184` 验证，ProviderClient Broker Composition v1 已由 CI run `29668410474` 验证，Official AI Component v1 已由 CI run `29671796776` 验证，Content Worker Composition v1 已由 CI run `29676464993` 验证。Production Bundle / Runtime v1 已完成本地 release E2E、缺失官方种子负向门禁、秘密扫描和一次有界内部评审，等待精确提交、推送及 CI 容器验证；生命周期 dispatcher、MCP transport、provider/content API 和 Reader AI UI 仍是后续依赖。

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
