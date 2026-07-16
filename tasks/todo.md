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

- [ ] Feed/subscription/entry/entry_state schema 与三数据库迁移。
- [ ] URL 安全、SSRF 防护、条件请求、解析、清洗和幂等插入。
- [ ] 将 `https://www.ithome.com/rss/` 作为 opt-in/live smoke，覆盖真实抓取、RSS 2.0 解析、HTML 清洗、`60-item` 级入库、二次刷新幂等无重复，以及列表/正文展示；live smoke 与确定性 CI fixture 分开。
- [ ] 调度租约、退避、保留策略和刷新事件。
- [ ] 订阅树、文章列表、阅读器、已读/未读/收藏。
- [ ] CommaFeed 效率内核：All/Unread/Starred、J/K 与 N/P、批量已读快照、下一未读来源、来源内搜索和非打扰式新文章合并。
- [ ] 列表重载与 Feed 网络抓取分离，queued/running/cooldown/partial failure 状态可见。

## 3. Multi-user organization

- [ ] 分类、排序、阅读游标和用户设置。
- [ ] 注册策略、管理员用户管理和跨用户隔离测试。

## 4. OIDC

- [ ] Authorization Code + PKCE + state + nonce。
- [ ] external identity、账号绑定、claim/domain/group 策略。
- [ ] OIDC 设置和账号 UI。

## 5. AI content

- [ ] provider/processor 接口和 OpenAI-compatible adapter。
- [ ] content jobs/artifacts、摘要、翻译、缓存、版本和额度。
- [ ] AI artifact UI 与 prompt injection 安全测试。

## 6. Plugin ecosystem

- [ ] WIT ABI、SDK、manifest/capability 和 Wasmtime sandbox。
- [ ] `before/fetched/entry.process/persisted/completed` 生命周期。
- [ ] outbox、幂等重试、熔断、管理 UI 和示例插件。

## 7. MCP

- [ ] 外部 MCP Streamable HTTP/受限 stdio client 和 tool allowlist。
- [ ] Raindrop `/mcp` 与 `raindrop mcp --stdio` server。
- [ ] scopes、token、用户隔离和协议契约测试。

## 8. Portability/import

- [ ] PostgreSQL/MySQL 仓储契约和 CI service tests。
- [ ] OPML 导入预览/提交与 OPML 导出。
- [ ] 设置 JSON 导入/导出、保留、备份和恢复演练。

## 9. Product UX

- [ ] ASTRYX `AppShell + Layout + TreeList + List/Item` 响应式三栏。
- [ ] `>=1100px` 三栏、`720–1099px` 两区、`<720px` 单任务深链接路由，并恢复订阅树/列表/正文滚动锚点。
- [ ] Reader 规范化实体状态保证树、列表、正文的已读/收藏/计数一致；新条目不自动重排当前队列。
- [ ] 摘要/翻译/plugin artifact 作为非阻塞 sidecar，原文默认且始终可读。
- [ ] light/dark/system、Kami 排版、中文/英文完整覆盖。
- [ ] Emil motion、键盘、screen reader、reduced-motion 和移动端验证。

## 10. Release

- [ ] 跟踪 SeaORM 依赖链中的 `proc-macro-error2 2.0.1` future-incompatibility；上游修复后升级，current-stable CI 预警 lane 在此之前保持 `continue-on-error`，Rust 1.94 MSRV gate 继续阻塞。
- [ ] CI 质量门、依赖审计和 OpenAPI drift。
- [ ] Linux/Windows/macOS binary 与 checksums。
- [ ] GHCR + 可选 Docker Hub 的 amd64/arm64 Docker workflow。
- [ ] 非 root 容器、healthcheck、README、配置和运维文档。
