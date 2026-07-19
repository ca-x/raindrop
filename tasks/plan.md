# Raindrop 总体实施路线

权威设计规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`。

当前详细计划：`docs/superpowers/plans/2026-07-19-official-ai-component-v1.md`。

当前进展：Refresh Observability v1 已由 CI run `29646491921` 验证，AI Provider Adapters v1 已由 CI run `29648253330` 验证，AI Provider Core v1 已由 CI run `29652502840` 验证，Content Jobs / Artifacts Core v1 已由 CI run `29655164960` 验证，Official AI Plugin Contract / Registry Core v1 已由 CI run `29657539274` 验证，Wasmtime Component Host Core v1 已由 CI run `29663971184` 验证，ProviderClient Broker Composition v1 已由 CI run `29668410474` 验证。Official AI Component Invocation Contract v1 已通过本地与 detached worktree 的 format、Clippy、定向测试和全量 Rust 测试。Official AI Component v1 已由 CI run `29671796776` 验证：Provider broker 支持受限动态 tool-plan schema family，capability session 从完整 host-issued binding 集合重构并授权 schema；no-WASI Rust guest 实现摘要、翻译、两阶段 MCP enrichment、FAIL_OPEN/FAIL_CLOSED 和 persisted lifecycle intents；相同 core Wasm 确定性 componentize，组件导入不含 `wasi:`，真实 hardened Wasmtime host 覆盖 direct/MCP/lifecycle。下一真实依赖是 content worker composition、artifact 终态提交、lifecycle dispatcher、MCP transport 和 Reader sidecar；在这些能力接通前，AI、插件生态和 MCP 总任务继续保持未完成。OIDC、OPML、排序/阅读游标和真实 `v*` Docker 发布 smoke 仍保留在后续任务。

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
