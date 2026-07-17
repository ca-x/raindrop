# Raindrop 设计规格

## 1. 目标

Raindrop 是一个参考 CommaFeed 产品能力、使用 Rust 重写的自托管多用户 RSS 阅读器。它面向个人、家庭和小型团队实例，强调低内存占用、单文件部署、可靠的高频 Feed 抓取，以及不依赖外部前端服务器的完整 Web 体验。

首个稳定版本必须满足：

- 本地账号和 OpenID Connect 登录并存，管理员可控制注册策略。
- 支持环境变量全自动初始化；未提供环境变量或配置文件时，通过安全的 Web 设置向导完成初始化。
- 默认使用 SQLite WAL，并可切换 PostgreSQL 或 MySQL。
- React/TypeScript Web 界面构建后嵌入 Rust 可执行文件。
- 支持订阅、分类、文章阅读状态、收藏、OPML 导入/导出、便携设置导入/导出。
- 通过唯一随首版交付的官方 AI 内容 Wasm 插件提供可配置摘要和翻译，并以可重建派生内容保存结果。
- 提供版本化、沙箱化的内容插件 host 与完整 Feed 生命周期合同，同时支持 MCP 客户端与 MCP 服务端；首版不开放第三方插件分发。
- 支持浅色、深色、跟随系统主题，以及中文和英文界面。
- GitHub Actions 自动验证、构建多平台二进制和发布多架构 Docker 镜像。

## 2. 已确认的产品边界

- 用户模式：多用户实例，包含普通用户和管理员。
- 本地认证不是 OIDC 的降级临时代码，而是一等登录方式。
- SQLite 是无需配置的默认数据库；PostgreSQL/MySQL 是运行时可选后端，不要求分别编译不同程序。
- “导入”包含 OPML 订阅导入，以及不含秘密的用户界面设置 JSON 导入。首版不直接读取 CommaFeed 数据库。
- 首版语言为 `zh-CN` 和 `en`，所有文案必须经过消息目录，禁止把新文案硬编码为单一语言。
- Web UI 是产品界面，不照搬 Kami 静态页面模板；只采用其排版语言和颜色纪律。
- AI 与插件本次直接纳入正式架构和交付阶段，不以空 trait 或未接线入口冒充支持；唯一业务插件固定为官方签名、随二进制打包的 `raindrop.ai-content` Wasm Component。
- `raindrop.ai-content` 必须通过未来第三方也会使用的版本化 WIT ABI、manifest、capability host、job、artifact 和 lifecycle outbox 运行，不得使用 native processor 或 route handler 直连模型的私有捷径。
- 首版不提供插件市场、远程/匿名安装、本地任意组件上传、custom JavaScript、多插件编排 UI 或第三方插件兼容承诺；通用表和合同保留，使未来开放生态成为 additive change。

## 3. 参考实现结论

CommaFeed 当前实现采用 Quarkus + React/TypeScript + Mantine，核心数据结构将 Feed 作为跨用户共享资源，将订阅和阅读状态作为用户资源。它还包含：

- 依据 `ETag`、`Last-Modified`、内容哈希与经验间隔进行抓取。
- Feed URL 和条目 GUID 的规范化哈希去重。
- 每个订阅、每篇条目一行阅读状态。
- 首次启动时创建管理员账号。
- OPML 导入/导出、主题、多语言和响应式布局。

Raindrop 保留这些有效边界，但会减少“每次抓取 × 每位订阅用户”的状态写放大，并把无环境变量初始化扩展成数据库、站点和认证向导。

## 4. 方案比较与内部决策

### 方案 A：模块化单体 + 统一 ORM（采用）

Rust 使用 Axum、Tokio 和 SeaORM；React 资源由同一程序提供。数据库差异收敛在仓储与迁移模块内。

优点：一个可执行文件、部署简单、三种数据库运行时切换、事务边界清晰。缺点：少数高性能查询需要按数据库方言提供经过测试的 SQL。

### 方案 B：SQLx + 每种数据库独立查询实现

优点：核心查询可在编译期验证，SQL 控制力最强。缺点：SQLite/PostgreSQL/MySQL 的占位符、返回行为和锁语义会造成大量重复查询及测试矩阵，明显增加首版维护成本。

### 方案 C：嵌入式 LSM 存储 + 外部 SQL 适配

优点：抓取写入可高度顺序化。缺点：多用户关系、筛选、分页、迁移和运维工具需要自行实现，并且无法真正共享一套 PostgreSQL/MySQL 行为。

内部决策：采用方案 A。RSS 工作负载不是只有追加写入，还包含用户隔离、分类、多条件未读分页和事务性状态更新；关系模型比自定义 KV 层更适合。通过追加型条目、稀疏状态和索引控制解决写入压力，而不是引入第二套存储系统。

## 5. 技术栈

### 后端

- Rust 2024 edition，最低支持 Rust 1.94；该下限来自 Wasmtime 46 的实际 MSRV。CI 使用稳定版和项目声明的最低版本。
- Axum：HTTP 路由、中间件和 WebSocket/SSE 入口。
- Tokio：异步运行时和抓取工作池。
- SeaORM + sea-orm-migration：SQLite、PostgreSQL、MySQL 的统一实体和迁移。
- reqwest + rustls：Feed/OIDC HTTP 客户端。
- feedparser-rs 0.5.5：RSS 0.90/0.91/0.92/1.0/2.0、Atom 0.3/1.0 与 JSON Feed 1.0/1.1 解析；关闭默认 `http` feature，抓取仍只经过 Raindrop 的 SSRF-safe transport，任何格式中的 DTD/ENTITY 仍由项目安全策略拒绝。
- openidconnect：Authorization Code + PKCE + nonce/state。
- tower-sessions 或等价的服务端会话抽象；会话令牌只以哈希形式存库。
- argon2：Argon2id 密码哈希。
- ammonia：文章 HTML 白名单清洗。
- rust-embed：嵌入 `web/dist`。
- Wasmtime Component Model：运行无环境权限的 WASI 内容插件。
- rmcp：MCP client/server，启用 Streamable HTTP 和 stdio transport。
- tracing：结构化日志，敏感字段禁止进入日志。

### 前端

- React 19 + TypeScript 7 + Vite 8。
- `@astryxdesign/core` 0.1.6 + `@astryxdesign/theme-neutral` 0.1.6：主要组件、布局、主题和无障碍交互。依赖使用精确版本；0.1.5 存在生产 `jsxDEV` 崩溃，禁止使用。
- `@astryxdesign/cli` 0.1.6：组件文档、页面模板、主题构建和升级 codemod；不进入生产 runtime。
- `@stylexjs/stylex` 0.19：只用于 ASTRYX 支持的 `xstyle` 布局逃生口和稳定主题表面，不重写组件内部样式。
- React Router 7、Redux Toolkit 2。
- Lingui 6：`zh-CN` / `en` 消息目录。
- Vitest + Testing Library；Playwright 覆盖关键端到端流程。

锁文件 `Cargo.lock` 与 `web/package-lock.json` 均提交。CI 只使用冻结安装。

## 6. 总体架构

```text
Browser
  ├─ embedded React UI
  └─ /api/v1 + /auth + /events
                 │
             Axum app
  ┌──────────────┼────────────────┐
  │              │                │
Auth/Setup    Feed domain      Admin/Settings
  │              │                │
  └──────────── repositories ─────┘
                 │
       SQLite / PostgreSQL / MySQL

Scheduler → feed.refresh.before/fetched + entry.process ABI seams
          → claim/fetch/parse/clean → idempotent transaction + outbox
          → feed.refresh.persisted/completed → lifecycle dispatcher
                                             → official AI Wasm → job intents

Manual API / Reader / MCP server → content orchestrator → official AI Wasm
                                         ├─ ai.generate_structured → provider core
                                         ├─ mcp.call_tool → MCP client core
                                         └─ versioned content artifacts

Raindrop MCP client ↔ external MCP servers
Raindrop MCP server ↔ user-authorized AI agents
```

程序是模块化单体。HTTP 层只负责认证、授权、边界验证和 DTO 转换；领域服务负责规则；仓储负责查询和事务；抓取器不直接持有用户会话或 HTTP 请求对象。

## 7. 项目结构

```text
Cargo.toml                     Rust 包和特性
Cargo.lock                     锁定 Rust 依赖
build.rs                       生产构建时检查并嵌入 web/dist
src/main.rs                    启动、信号和退出码
src/app.rs                     依赖组装和 Axum Router
src/config/                    环境变量、TOML、向导配置与优先级
src/db/                        连接、事务、方言能力和仓储
src/migration/                 可移植迁移
src/api/                       /api/v1 DTO、路由和统一错误
src/auth/                      本地账号、OIDC、会话、CSRF、RBAC
src/setup/                     首次启动状态机和一次性令牌
src/feeds/                     URL 规范化、抓取、解析、调度、保留策略
src/content/                   内容编排、AI 作业、执行记录和派生产物
src/plugins/                   WIT ABI、bundled 官方组件、Wasmtime host、权限和生命周期事件
src/mcp/                       MCP client/server、transport、scope 和工具
src/import/                    OPML 与设置 JSON 导入/导出
src/web/                       嵌入资源、SPA fallback、安全响应头
tests/                         Rust 集成测试
web/src/                       React 应用
web/src/app/                   provider、router、store 和应用 bootstrap
web/src/features/setup/        设置向导页面、API、schema 和局部组件
web/src/features/auth/         本地/OIDC 登录与账号绑定
web/src/features/reader/       订阅树、文章列表、正文与阅读状态
web/src/features/settings/     用户外观、阅读和账号设置
web/src/features/admin/        用户、系统、AI provider、官方插件策略和 MCP 管理
web/src/features/ai/           摘要、翻译、artifact 与可选聊天交互
web/src/features/plugins/      官方 AI 插件查询、启停、配置、执行记录和重试
web/src/features/mcp/          外部连接、tools 和 token 管理
web/src/shared/                typed API、i18n、主题和跨 feature 组合件
web/e2e/                       Playwright 测试
.github/workflows/             CI、binary、Docker
docs/                          规格、计划和运维文档
```

## 8. 配置与初始化

### 8.1 配置优先级

从高到低：

1. `RAINDROP_*` 环境变量。
2. `--config <path>` 指定的 TOML。
3. `RAINDROP_DATA_DIR/config.toml`。
4. 安全默认值。

未知环境变量不影响启动；已识别但值非法的变量必须使启动失败，并报告变量名和允许格式，不回显秘密值。

### 8.2 环境变量初始化

最小自动启动配置：

```text
RAINDROP_DATABASE_URL=sqlite://data/raindrop.db?mode=rwc
RAINDROP_PUBLIC_URL=https://rss.example.com
RAINDROP_SESSION_SECRET=<至少 32 字节随机值>
RAINDROP_BOOTSTRAP_ADMIN_USERNAME=admin
RAINDROP_BOOTSTRAP_ADMIN_PASSWORD=<secret>
```

PostgreSQL/MySQL 通过 URL scheme 自动选择。OIDC 使用：

```text
RAINDROP_OIDC_ISSUER_URL=
RAINDROP_OIDC_CLIENT_ID=
RAINDROP_OIDC_CLIENT_SECRET=
RAINDROP_OIDC_SCOPES=openid,email,profile
```

若数据库无用户且完整的 bootstrap admin 环境变量存在，程序在事务内创建管理员；之后不会再次读取该密码创建或覆盖账号。

### 8.3 无环境变量设置向导

当数据库配置环境变量和配置文件都不存在时，程序进入 `SetupRequired` 状态，只开放静态资源、健康检查和 `/api/v1/setup/*`。

向导步骤：

1. 语言与站点 URL。
2. 数据库：默认 SQLite，也可输入 PostgreSQL/MySQL URL并测试连接。
3. 管理员账号。
4. 可选 OIDC 与注册策略。
5. 校验摘要并写入配置。

安全约束：

- 启动时生成一次性 setup token 并打印到终端；远程请求必须提供该 token。
- loopback 请求可免 token，反向代理场景不得仅依赖来源 IP。
- token 完成后立即失效，重启时重新生成。
- `config.toml` 原子写入，Unix 权限为 `0600`。
- 数据库连接测试和 OIDC discovery 有严格超时，错误不包含密码或 client secret。
- 完成向导后在同一进程重新加载配置、运行迁移并切换到正常 Router；无需手工重启。

若环境变量已提供数据库配置但未提供管理员，则仅开放“创建首位管理员”的精简向导，不允许 Web UI 修改环境托管的数据库或会话配置。

## 9. 数据库设计与 DDIA 内部评审

### 9.1 工作负载

- 网络抓取并发高，但每个 Feed 的数据库提交很小且可序列化。
- 条目以追加为主；Feed 抓取状态是热点更新行。
- 文章列表是按用户、分类/Feed、未读/收藏和时间倒序的 OLTP 查询。
- 同一 Feed 可被多个用户共享，不能为每次抓取给每个用户复制文章正文。
- “全部标为已读”必须是 O(1) 或 O(订阅数)，不能随着历史条目数量线性增长。

### 9.2 核心表

#### 身份与配置

- `users(id, username, email, password_hash, is_disabled, created_at, last_login_at)`
- `user_roles(user_id, role)`；唯一键 `(user_id, role)`。
- `external_identities(id, user_id, provider_key, subject, email_at_link)`；唯一键 `(provider_key, subject)`。
- `sessions(id_hash, user_id, csrf_hash, created_at, last_seen_at, expires_at)`。
- `user_settings(user_id, locale, color_scheme, accent, reading_mode, reading_order, density, updated_at)`。

#### 订阅与文章

- `feeds(id, source_url, normalized_url, normalized_url_hash, site_url, title, etag, last_modified, content_hash, last_fetched_at, next_fetch_at, error_count, disabled_until, lease_owner, lease_until)`。
- `categories(id, user_id, title, position)`；唯一键 `(user_id, title)`。
- `subscriptions(id, user_id, feed_id, category_id, title_override, position, created_at, read_before, filter_expression)`；唯一键 `(user_id, feed_id)`。
- `entries(id, feed_id, identity, identity_hash, canonical_url, title, author, sanitized_content, summary, published_at, inserted_at, updated_at, content_hash, direction, enclosure_json)`；唯一键 `(feed_id, identity_hash)`。
- `entry_states(user_id, entry_id, is_read_override, is_starred, updated_at)`；主键 `(user_id, entry_id)`。
- `entry_tags(user_id, entry_id, tag)`；主键 `(user_id, entry_id, tag)`。

#### AI、插件与 MCP

- `ai_providers(id, owner_user_id, kind, endpoint, model, encrypted_secret, config_json, is_enabled)`；实例级 provider 的 `owner_user_id` 为空。
- `content_jobs(id, user_id, entry_id, operation, target_locale, trigger, plugin_key, plugin_version, component_digest, provider_binding_id, input_hash, config_hash, idempotency_key, call_chain_id, remaining_depth, status, attempts, next_attempt_at, created_at, completed_at)`；唯一 `(user_id, idempotency_key)`。
- `content_job_attempts(id, job_id, attempt, status, error_code, retryable, provider_request_count, mcp_call_count, input_tokens, output_tokens, estimated_cost_micros, execution_metadata_json, started_at, completed_at)`。
- `content_artifacts(id, user_id, entry_id, job_id, kind, locale, schema_id, input_hash, config_hash, processor_key, processor_version, provider_label, payload_json, provenance_json, created_at)`；唯一键覆盖完整 artifact identity。
- `plugin_installations(id, plugin_key, version, abi_version, distribution, component_digest, manifest_json, signature_key_id, signature, system_state, installed_at, updated_at)`；首版只存在 `raindrop.ai-content`，且 `distribution = BUNDLED_OFFICIAL`。
- `plugin_configs(id, plugin_id, owner_user_id, schema_version, config_json, config_hash, is_enabled, revision, created_at, updated_at)`；唯一 `(plugin_id, owner_user_id)`，配置不含 secret。
- `plugin_capability_grants(id, plugin_id, owner_user_id, capability, operation, resource_type, resource_id, constraints_json, revision, created_at, revoked_at)`。
- `plugin_kv(plugin_id, owner_user_id, key, value, updated_at)`；作为通用 ABI 预留并有容量配额，官方 AI 插件首版不申请。
- `lifecycle_outbox(id, event_type, aggregate_id, payload_json, idempotency_key, available_at, attempts, status, created_at)`。
- `lifecycle_deliveries(event_id, plugin_id, owner_user_id, config_hash, status, attempts, last_error_code, completed_at)`；唯一键 `(event_id, plugin_id, owner_user_id, config_hash)`。
- `mcp_connections(id, owner_user_id, name, transport, endpoint, encrypted_credentials, tool_allowlist, tool_effect_policy, is_enabled)`。
- `mcp_tool_calls(id, job_attempt_id, user_id, call_chain_id, connection_id, tool_name, tool_schema_digest, arguments_redacted_json, arguments_digest, result_redacted_json, result_digest, status, error_code, duration_ms, created_at)`。
- `api_tokens(id_hash, user_id, name, scopes, created_at, last_used_at, expires_at, revoked_at)`，供 MCP 和外部 API 使用。

正文与条目放在同一行，因为阅读路径总是同时取标题与正文，拆表会增加常态 join；抓取热点字段位于 `feeds`，不会反复改写大正文页。

### 9.3 稀疏阅读状态

有效已读状态：

```text
entry_states.is_read_override
  ?? (entries.inserted_at <= subscriptions.read_before)
  ?? (entries.inserted_at < subscriptions.created_at)
```

单篇标记写入或更新一行 `entry_states`。全部标为已读只推进订阅的 `read_before`；如果用户把旧文章重新标为未读，则写 `is_read_override = false`。收藏状态与阅读状态共用稀疏行。

这避免 CommaFeed 风格的 `新条目数 × 订阅用户数` 批量状态写入，并保持用户可见行为一致。

### 9.4 幂等与事务

- Feed URL 规范化后使用 BLAKE3 哈希索引，同时比较完整规范化 URL 防止哈希碰撞。
- 条目标识优先级：有效 GUID → 规范化 canonical URL → 稳定内容指纹。
- `(feed_id, identity_hash)` 唯一约束是最终去重边界；重复抓取使用 insert-on-conflict-ignore/等价实现。
- 每次 Feed 更新在一个短事务中写入新条目并更新抓取元数据。
- 网络请求、XML/JSON 解析、HTML 清洗全部在事务外完成。
- 事务重试必须安全；外部通知只能在事务提交后发送。

### 9.5 调度与租约

工作器先查询到期 Feed，再执行条件更新：仅当 `lease_until` 已过期时写入随机 `lease_owner` 和新租约。受影响行数为 1 才获得抓取权。该 CAS 模式在三种数据库中保持一致，避免依赖 `SKIP LOCKED`。

SQLite：

- 启用 WAL、`foreign_keys=ON`、`busy_timeout` 和合理同步级别。
- 默认数据库写并发为 1；HTTP 获取仍可并发。
- 禁止把 SQLite 数据文件放在不支持可靠文件锁的网络文件系统上。

PostgreSQL/MySQL：默认写并发可提高，但同一 Feed 仍由租约保证单工作器更新。

### 9.6 关键索引

- `feeds(next_fetch_at, disabled_until, lease_until)`。
- 唯一 `feeds(normalized_url_hash)`，碰撞时由应用比较完整 URL。
- 唯一 `entries(feed_id, identity_hash)`。
- `entries(feed_id, inserted_at DESC, id DESC)`。
- `subscriptions(user_id, category_id, position)`。
- `entry_states(user_id, is_starred, updated_at DESC)`。
- `entry_tags(user_id, tag, entry_id)`。
- `sessions(expires_at)`。
- `content_jobs(status, next_attempt_at)`。
- `content_artifacts(user_id, entry_id, kind, locale, created_at DESC)`。
- `lifecycle_outbox(status, available_at)`。

不为低选择性的布尔列单独建索引。索引必须由真实列表查询和 `EXPLAIN` 结果证明，不预先建立全文索引。

### 9.7 一致性、故障与演进结论

- 单主关系数据库提供所需的 read-after-write；首版不引入跨区域复制或事件总线。
- 所有用户写操作在单数据库事务中完成，不使用分布式事务。
- 唯一约束承担用户名、OIDC subject、订阅和条目标识的线性化仲裁。
- 数据库是记录系统；未读计数、Feed 树等可重建结果是派生数据，不成为第二事实源。
- 原始 Feed/entry 是记录系统；摘要、翻译和未来插件标注/向量等结果全部是带输入、配置、组件和 provider 版本的派生数据，可安全重算。
- 生命周期后置插件通过事务 outbox 至少一次投递。系统不宣称端到端 exactly-once；event、delivery、job 和 artifact 分别有稳定 idempotency key/唯一约束。
- Feed 持久化事务只写 core 数据和 outbox；模型、MCP 和 Wasm post-commit handler 均在提交后运行，任何失败不回滚条目。
- 迁移只做向后兼容的加列/建表/回填/切换；破坏性删除至少跨一个发布版本。
- 定期备份不等于可恢复，运维文档必须包含 SQLite 与外部数据库的恢复演练命令。

## 10. Feed 抓取

- 默认最短间隔 5 分钟，最大退避 4 小时。
- 尊重 `ETag`、`Last-Modified`、`Cache-Control` 和 `Retry-After`。
- 根据最近发布时间估计活跃度，但不得低于最短间隔。
- 每个响应有连接、TLS、首包、总耗时和最大正文尺寸限制。
- 重定向每跳重新执行 SSRF 校验，最多 5 跳。
- 默认拒绝 loopback、链路本地、私网、保留地址和云元数据地址；管理员可显式开启“允许内网 Feed”，界面必须说明风险。
- DNS 解析结果固定到本次连接，防止校验后的 DNS rebinding。
- XML 不允许外部实体；压缩响应同时限制解压后大小。
- 保存的正文先经 HTML 清洗；React 不直接渲染未经清洗的 Feed HTML。

## 11. 认证、授权与威胁模型

### 11.1 本地账号

- 密码使用 Argon2id，参数在首次启动时校准到约 250ms，并设置安全下限。
- 用户名大小写归一化；展示名与登录标识分离。
- 登录、注册、密码重置和 setup 端点分别限流。
- 认证失败返回统一信息，避免枚举用户名或邮箱。

### 11.2 OIDC

- Authorization Code Flow + PKCE S256 + state + nonce。
- issuer 必须与 discovery 文档和 ID Token 完全匹配。
- `(provider_key, subject)` 是账号绑定主键；默认不凭未验证邮箱自动合并账号。
- 只有 `email_verified=true` 且管理员启用邮箱自动链接时才允许链接，冲突返回可审计错误。
- 管理员可配置允许注册、允许域名和管理员 group/claim 映射。
- OIDC client secret 不进入前端 bootstrap、日志、导出文件或错误正文。

### 11.3 会话和 CSRF

- 浏览器 Cookie 只保存 256-bit 随机会话令牌；数据库只保存令牌哈希。
- CSRF token 使用 BLAKE3 域分离从会话令牌单向派生，数据库仅保存其哈希；页面重载和多标签页得到同一 token，泄漏 CSRF token 不能反推 HttpOnly 会话令牌。
- Cookie：`HttpOnly`、`SameSite=Lax`，HTTPS 时 `Secure`，固定 Path，不设置宽泛 Domain。
- 修改操作要求 CSRF token，并校验 Origin/Host。
- 会话 last-seen 最多每 15 分钟触碰一次，避免每个 API 请求写数据库。
- 退出、禁用用户、密码变更和管理员撤销都能使相关会话失效。

### 11.4 授权

- 所有仓储查询必须显式接收 `UserId` 或在管理员专用模块中运行。
- 普通用户不可通过猜测 ID 读取其他用户的分类、订阅、状态、设置和导入任务。
- 管理端点同时检查会话和 `ADMIN` role，不能只依赖前端隐藏入口。

### 11.5 STRIDE 摘要

| 边界 | 主要滥用 | 控制 |
| --- | --- | --- |
| 登录/OIDC 回调 | 冒充、重放、登录 CSRF | PKCE、state、nonce、短期事务 Cookie、限流 |
| 首次设置 | 抢占首位管理员 | 一次性 setup token、状态机、完成后关闭端点 |
| Feed URL | SSRF、DNS rebinding、压缩炸弹 | 地址分类、固定解析、逐跳校验、大小和时间限制 |
| OPML/设置导入 | XML/JSON DoS、越权覆盖 | 大小/条目上限、流式解析、schema 验证、用户作用域事务 |
| 文章 HTML | 存储型 XSS、跟踪资源 | 服务端清洗、CSP、可选图片代理 |
| 多用户 API | IDOR、权限提升 | 每条查询用户作用域、管理员角色检查、集成测试 |
| AI 处理 | prompt injection、秘密泄露、成本耗尽 | Feed/模型/MCP 输出视为不可信、只经 `ai.generate_structured`、secret 不过 ABI、配额与超时 |
| 插件 | 越权文件/网络访问、死循环、供应链投毒 | 只加载 bundled official signed component、无 ambient WASI、capability broker、fuel/内存/超时/digest/signature |
| MCP client | 恶意工具描述、越权工具调用、递归环路 | 用户 + operation + connection + exact tool allowlist、AI 自动任务只读、call-chain/depth、schema/大小/成本限制 |
| MCP server | token 泄露、跨用户读取、滥用 AI 额度 | 哈希 token、最小 scope、用户作用域、限流、审计 |

## 12. API 契约

API 基础路径为 `/api/v1`。资源使用复数名词，字段使用 camelCase，枚举使用 `UPPER_SNAKE_CASE`。列表从首版起使用游标分页，不暴露不稳定页码语义。

统一错误：

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Request validation failed",
    "fields": { "databaseUrl": "unsupported scheme" },
    "requestId": "01J..."
  }
}
```

错误消息不包含 SQL、栈、文件路径或秘密。核心端点：

```text
GET    /api/v1/bootstrap
GET    /api/v1/health/live
GET    /api/v1/health/ready

POST   /api/v1/setup/database-check
POST   /api/v1/setup/oidc-check
POST   /api/v1/setup/complete

POST   /api/v1/auth/login
POST   /api/v1/auth/logout
GET    /api/v1/auth/session
GET    /api/v1/auth/oidc/:provider/start
GET    /api/v1/auth/oidc/:provider/callback

GET    /api/v1/feeds?cursor=&limit=&categoryId=&state=
POST   /api/v1/subscriptions
PATCH  /api/v1/subscriptions/:id
DELETE /api/v1/subscriptions/:id
POST   /api/v1/subscriptions/:id/refresh

GET    /api/v1/entries?cursor=&limit=&feedId=&categoryId=&state=
PATCH  /api/v1/entries/:id/state
POST   /api/v1/entries/mark-read

GET    /api/v1/categories
POST   /api/v1/categories
PATCH  /api/v1/categories/:id
DELETE /api/v1/categories/:id

POST   /api/v1/imports/opml
GET    /api/v1/exports/opml
POST   /api/v1/imports/settings
GET    /api/v1/exports/settings

GET    /api/v1/admin/users
PATCH  /api/v1/admin/users/:id
GET    /api/v1/admin/system

GET    /api/v1/ai/providers
POST   /api/v1/ai/providers
POST   /api/v1/entries/:id/summaries
POST   /api/v1/entries/:id/translations
GET    /api/v1/entries/:id/artifacts

GET    /api/v1/plugins
GET    /api/v1/plugins/raindrop.ai-content
PATCH  /api/v1/plugins/raindrop.ai-content/state
GET    /api/v1/plugins/raindrop.ai-content/config
PUT    /api/v1/plugins/raindrop.ai-content/config
GET    /api/v1/plugins/raindrop.ai-content/executions
GET    /api/v1/plugins/raindrop.ai-content/executions/:id
POST   /api/v1/plugins/raindrop.ai-content/executions/:id/retry

GET    /api/v1/mcp/connections
POST   /api/v1/mcp/connections
GET    /api/v1/mcp/tools
POST   /api/v1/tokens
DELETE /api/v1/tokens/:id
```

首版 `GET /plugins` 只返回官方 `raindrop.ai-content` descriptor（全局禁用时仍返回 disabled state），不提供 install、uninstall、upload、catalog 或 marketplace endpoint。插件配置使用 manifest 内的 JSON Schema 验证并以 revision/`If-Match` 防止丢失更新；config 只引用 provider、connection 和 exact tool ID，不保存 secret。未来分发能力通过新增 endpoint additive 引入。

OpenAPI JSON 由后端生成并在 CI 中校验。前端 DTO 从 OpenAPI 生成或由共享 schema 生成，禁止手工维护两套漂移类型。

## 13. Web UI 与 UX

### 13.1 信息架构

桌面采用三栏：订阅树、文章列表、阅读正文。窄屏采用堆栈导航，并保留浏览位置。主流程：登录/设置 → 全部未读 → 文章 → 下一篇；添加订阅和导入是次要流程。

响应式形态：

- `>= 1100px`：订阅树、文章列表、阅读正文三栏。
- `720px–1099px`：文章列表与阅读正文两区；订阅树通过明确的“来源”按钮打开可访问 drawer，关闭后焦点回到触发器。
- `< 720px`：订阅 → 列表 → 阅读的单屏堆栈路由；浏览器返回和界面返回行为一致，并恢复列表位置与文章阅读位置。

Reader 体验采用“CommaFeed 的效率内核 + 现代化增强”，不做视觉复刻，也不把 AI 摘要替换成默认首页：

- 保留 All、Unread、Starred、分类、单 Feed、来源内搜索、未读聚合和当前来源稳定保留等成熟心智模型。
- 保留连续清空队列的操作语法：`J/K` 打开并标记上一篇/下一篇，`N/P` 只移动选择，`M` 切换已读，`S` 收藏，`Shift+J/K` 进入上一/下一未读来源，`mod+K` 快速跳转来源。
- 批量已读基于用户确认时的稳定快照；操作期间新到达且尚未展示的条目继续保持未读。单篇变为已读后，当前行在用户进入下一篇前不因未读过滤立即消失。
- 新文章到达只更新计数并显示“载入 N 篇新文章”，不得自动改变当前选择、滚动锚点或正文；用户明确合并后才更新队列。
- 明确区分“重新加载数据库中的列表”和“请求服务端抓取 Feed”，分别表达 queued、running、cooldown、partial failure 与上次成功时间。
- 原文始终是默认阅读内容；摘要、翻译和插件 artifact 是阅读器的次级 sidecar，失败或未配置时不能阻塞正文、已读/收藏和上一篇/下一篇。

Reader 核心路由保持深链接与跨端同义：

```text
/reader/all?state=unread|all
/reader/starred
/reader/category/:categoryId?state=unread|all
/reader/feed/:feedId?state=unread|all
/reader/all/entry/:entryId
/reader/starred/entry/:entryId
/reader/category/:categoryId/entry/:entryId
/reader/feed/:feedId/entry/:entryId
```

桌面选择 `entryId` 时继续显示三栏；移动端同一 URL 渲染独立阅读详情。来源/过滤/排序/搜索可恢复，返回时按 `entry id + offset` 恢复列表锚点，再恢复正文位置。`selected`、`opened`、`read`、`starred` 是不同状态，不能互相隐式覆盖。

页面：

- 首次设置向导、登录/注册/OIDC。
- 全部、未读、收藏、分类、单 Feed 文章列表。
- 阅读正文与外部打开。
- 添加订阅、OPML 导入。
- 外观、阅读、账号、OIDC 绑定设置。
- 管理员用户和系统状态。
- 文章摘要/翻译 sidecar、AI provider 与额度设置。
- 只展示官方 AI 内容插件的来源/版本、启停、配置、自动触发范围、MCP enrichment、执行记录和失败重试。
- MCP 外部连接、工具 allowlist 和供 AI agent 使用的访问 token 管理。

### 13.2 ASTRYX 组件边界

ASTRYX 是默认组件来源。开始自定义控件前必须运行：

```bash
node web/node_modules/@astryxdesign/core/docs.mjs --list --brief
node web/node_modules/@astryxdesign/core/docs.mjs ComponentName
npx --prefix web astryx template --list
```

Raindrop 的组件映射：

- 根布局：一个 `AppShell`；桌面三栏使用一个 `Layout`，`LayoutPanel + Resizable` 提供可键盘调整的订阅、列表和阅读面板。
- 移动端：使用 `AppShell + MobileNav` 和同一 feature 组件的窄屏组合，不维护第二套业务状态或 API client。
- Feed/分类层级：`TreeList`。`SideNav` 只用于设置/管理等路由导航，不能当内容筛选树。
- 文章列表：`List + Item`，不使用 `Table`；用户、官方插件 execution、MCP 审计等均匀列数据才使用 `Table` 和所需 hooks。
- 设置和向导：`Section`、`FormLayout`、`TextInput`、`Selector/RadioList`、`FileInput`、`Banner`、`ProgressBar`、`Button`。
- 反馈：空态 `EmptyState`，尺寸已知加载 `Skeleton`，未知加载 `Spinner`，持久问题 `Banner`，短时确认 `Toast`。
- 危险确认：`AlertDialog`；表单弹层 `Dialog purpose="form"`；锚定交互 `Popover`。
- 快捷键：`useHotkeys + Kbd`；命令入口使用 `CommandPalette`。
- AI/MCP 会话：`ChatLayout`、`ChatMessageList`、`ChatComposer`、`ChatToolCalls`、`Markdown` 和 `useStreamingText`。

只允许在三种情况下写业务 CSS：ASTRYX 没有表达的领域布局、文章正文排版、或已复现且组件 props 无法解决的缺陷。颜色、间距、圆角、状态和基础排版优先走 ASTRYX tokens；禁止复制 ASTRYX 内部 DOM/CSS 来制造“定制组件”。

### 13.3 Emil 交互规则

- 键盘导航、切换下一篇等高频操作不使用进出场动画。
- 按钮按下使用 100–160ms 的轻微 `scale(0.97)` 反馈。
- popover 从触发点展开，modal 保持中心 origin。
- dropdown/tooltip 控制在 125–200ms，并使用强 `ease-out`。
- 只动画 transform 和 opacity；动态列表优先可中断 transition。
- hover 只在精细指针设备启用。
- `prefers-reduced-motion` 下移除空间移动，保留必要淡入和颜色反馈。
- loading 不能阻塞已经可读的旧内容；刷新时保留列表并显示局部状态。
- ASTRYX 已提供的 keyboard、focus restore、live-region 和 reduced-motion 行为不得被业务 wrapper 破坏。

### 13.4 Kami 排版规则

- 浅色主题使用温暖纸色背景，主强调色为墨蓝；深色主题使用暖黑而非纯黑。
- 导航和控件使用系统 sans 字体；文章标题、正文阅读面板采用 serif-led 字体栈。
- 中文 serif：`Source Han Serif SC, Noto Serif CJK SC, Songti SC, serif`。
- 英文 serif：`Charter, Georgia, serif`。
- 不捆绑商业字体；标题、正文、说明建立稳定字号和行高层级。
- 阅读正文最大行宽约 72 个拉丁字符；CJK 正文使用更宽松行高和避头尾标点规则。
- 一个界面只使用一个主强调色，状态色仅承担错误、警告、成功语义。
- 基于 `theme-neutral` 构建 Raindrop 主题；运行时 accent 通过 ASTRYX 派生 token 改变，不硬编码第二套色板。

### 13.5 主题与多语言

- `LIGHT`、`DARK`、`SYSTEM` 三种模式，服务端保存，首屏通过内联 bootstrap 防止闪烁。
- 用户可选择有限的无障碍 accent 预设；每个预设必须通过对比度检查。
- `zh-CN` 和 `en` 均覆盖前端、邮件/通知和 API 可展示消息。
- 使用逻辑 CSS 属性支持未来 RTL；Feed 正文依据条目 direction 独立渲染。

### 13.6 前端模块和文件规模

- 按 `features/setup|auth|reader|settings|admin|ai|plugins|mcp` 拆分；每个 feature 再分 `api`、`model`、`components`、`pages`、`messages`。
- 页面只组装 feature 组件，不直接实现请求、schema、状态机和复杂列表逻辑。
- 单个 TypeScript/TSX 文件默认不超过约 250 行；接近上限时按职责拆分，而不是压缩格式规避。
- 一个文件只导出一个主要页面或复杂组件；可复用的小型纯类型/常量除外。
- 禁止形成巨型 `App.tsx`、`client.ts`、`types.ts`、`store.ts` 或万能 `utils.ts`。
- ASTRYX primitive 的领域组合可以封装，例如 `FeedTree`、`EntryListItem`，但禁止再次封装同名通用 `Button`、`Dialog`、`Selector`。

### 13.7 移动端适配

视觉验收基准：`docs/prototypes/mobile-reader-detail.html`。它只用于移动阅读详情的结构、触控和排版审查；生产实现仍必须使用 ASTRYX 组件和真实应用状态。

- 设计基准覆盖 390×844、360×800 和 768px 宽度；主要操作在首屏可达。
- 交互目标至少 44×44 CSS px，正文至少 14px，列表副文案不得低于 13px。
- 使用 `100dvh` 与 `env(safe-area-inset-*)`，避免地址栏和刘海遮挡；滚动属于当前内容区，导航框架不跟随正文滚走。
- 阅读详情是单一任务屏，不保留桌面三栏缩小版；显示明确返回、收藏、已读、摘要/翻译和外部打开动作。
- Feed 列表是单一任务屏，保留未读筛选与刷新状态；不把设置、导入和正文硬塞进同屏。
- 不依赖 hover 或右键；上下文动作必须有可发现的触控入口。ASTRYX `ContextMenu` 可作为增强，但不是唯一入口。
- 高频的返回、下一篇、标记已读不做进出场动画；`MobileNav`/偶发 drawer 使用可中断、低于 300ms 的移动并遵循 reduced-motion。
- 不自造 swipe-back 或下拉刷新物理引擎；只有在标准导航和 ASTRYX 能力不足且真实设备验证收益明确时才增加手势。
- 中文长标题、英文长 URL、RTL Feed、动态字号 200% 和横屏均不得造成水平页面滚动。
- 所有核心动作必须有显式按钮；swipe、long-press、右键和双击只作为效率增强，不能成为已读、收藏、来源编辑或返回的唯一入口。
- 从阅读详情返回时焦点和滚动恢复到原列表项；从列表返回时恢复订阅树位置。dialog/drawer 关闭后焦点回到触发器。
- AI 摘要、翻译等派生结果从阅读器次级 action sheet/inspector 进入；移动详情不放底部主 tab bar，不让官方插件 artifact 冒充 Feed 原文。

## 14. 导入与导出

### OPML

- 文件上限 10 MiB，最多 10,000 个 outline。
- 流式解析，不解析外部实体。
- 先生成预览：有效、重复、非法 URL、预计新增数量。
- 提交时按用户事务导入；单个源抓取失败不回滚其余有效订阅。
- 导出保留分类和自定义标题。

### 设置 JSON

- 带 `schemaVersion`，只接受已知字段并允许向前兼容地忽略未知可选字段。
- 仅包含 locale、主题、阅读偏好和布局密度。
- 永不包含密码哈希、会话、API key、OIDC secret、数据库 URL 或管理员策略。

## 15. AI 内容能力

详细且绑定的 ABI、manifest、配置、错误、限制和测试设计见 `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`；本节只保留总规格必须维持的产品与模块合同。

### 15.1 内容编排与官方插件

首版 `SUMMARIZE` 和 `TRANSLATE` 都由官方 `raindrop.ai-content` Wasm Component 实现。HTTP route、Reader 和 Raindrop MCP server 只调用 content orchestrator；orchestrator 负责用户/条目授权、job/idempotency、配置快照、组件版本、artifact 和 retry，不在 route handler 或 native processor 中直接调用模型。

手动入口可立即排队。自动入口只消费 `feed.refresh.persisted` post-commit event，按用户配置返回 declarative job intents；host 重新授权并幂等创建 job。Feed 抓取、解析、清洗和持久化事务内不得调用 Wasm post-commit handler、provider 或 MCP。

`SUMMARIZE` 输出版本化 summary JSON；`TRANSLATE` 输出目标 locale、title 和安全 Markdown。两者都必须通过 artifact JSON Schema 和大小限制，不覆盖 `entries.title`、`entries.summary` 或 `entries.sanitized_content`；UI 始终允许回到原文。

### 15.2 Provider

`AiProvider` 是 core host 内部 adapter，负责模型调用、能力描述、结构化响应和计量：

```rust
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    fn capabilities(&self) -> AiCapabilities;
    async fn generate(&self, request: AiRequest) -> Result<AiResponse, AiError>;
}
```

首版固定提供四类 HTTP adapter：Anthropic Messages-compatible（Claude 格式）、OpenAI Responses、OpenAI Chat Completions-compatible，以及 Google Gemini native API。Chat Completions-compatible adapter 可连接兼容网关和本地服务；其它 adapter 也允许管理员配置经过 URL policy 校验的兼容 base URL。provider/plugin 接口不暴露供应商专有 DTO，四类响应统一映射为 canonical structured generation、usage、finish reason 和 provider-safe error。后续 provider 只新增 adapter。

Provider 可由管理员设置为实例共享，或由用户设置为个人 provider。secret 使用实例 master key 加密；API 响应只能返回 `isSecretConfigured`。

插件只能调用 host-brokered `ai.generate_structured`，提交 host-issued provider binding、untrusted input、output JSON Schema 和剩余预算内的 token 请求。host 重新检查 provider 授权、quota、并发、token、成本和 timeout，再返回 validated JSON 与非秘密 usage；secret、请求 header 和原始 transport 永不穿过 WIT ABI。

### 15.3 任务、缓存与成本

- AI 请求进入 `content_jobs`，Web 请求返回 job/artifact 状态，不占用长 HTTP 或数据库事务。
- 唯一输入由用户、entry content hash、operation/locale、plugin/component、provider/model、prompt/schema、config 和 MCP provenance 版本构成；相同输入复用 artifact。
- 条目正文变化后旧 artifact 保留审计元数据但不再作为 current result。
- provider 支持并发、每分钟请求、token、每 job cost ceiling 和每用户日额度；失败采用最多 3 attempts 的有界指数退避。
- 可按手动 Reader/API/MCP server 或配置的 Feed/分类/全部已订阅 Feed 新条目触发；自动触发默认关闭，不对历史条目回填。
- 本地推理未来可实现为新 provider；若加载本地模型，必须单例复用并批处理，禁止每次请求重新加载。

### 15.4 Prompt injection 边界

- Feed 标题/正文、模型输出、MCP tool description/result 明确放入 untrusted data 字段，不能改变 system policy、capability grant 或 output schema。
- 摘要和翻译默认没有 MCP；组件永远没有文件、环境、进程、数据库、任意网络或 raw MCP transport 权限。
- 模型输出按结构化 schema 解析；解析失败不进入 DOM、SQL、shell、路径、provider/MCP 配置或插件权限。
- MCP enrichment 必须按 operation 独立开启，并同时通过用户、connection、exact tool、只读 effect、调用/depth/token/cost 预算和审计；失败按配置 fail-open 回退无工具生成或 fail-closed 结束 job，但不影响 Feed/原文。

## 16. 内容插件系统

### 16.1 首版运行模型与分发边界

唯一随首版交付的插件是官方签名、嵌入二进制的 `raindrop.ai-content` Wasm Component。release build 和启动都校验 canonical manifest、component digest、签名、`pluginKey` 与 ABI；篡改或不兼容进入 quarantine/fail closed。即使官方组件与 host 同仓库，也必须通过未来第三方相同的 `raindrop:content-plugin@1` WIT world 和 capability broker，不允许 native shortcut。

首版没有插件市场、catalog、远程/匿名安装、本地任意 Wasm 上传、未签名 sideload、custom JavaScript、多插件排序或编排 UI。通用 installation/config/grant/KV 表、manifest/WIT/event schema 和 fixture 保留；未来安装与生态通过新 endpoint 和新 UI additive 增加。

### 16.2 生命周期

首版稳定事件名称：

```text
feed.refresh.before
feed.refresh.fetched
entry.process
feed.refresh.persisted
feed.refresh.completed
```

- `feed.refresh.before`：网络请求前，可返回 host-validated skip/retry hint 与 safe header patch；不能绕过 URL/SSRF。
- `feed.refresh.fetched`：限制响应大小后、解析前，可返回 accept/reject、诊断或 parser intent。
- `entry.process`：规范化/清洗后、持久化前，可返回 bounded patch/drop/annotations；host 重算和验证 identity/正文。
- `feed.refresh.persisted`：核心事务提交后，含新增/更新 entry refs、计数、content hashes 和稳定 event ID。
- `feed.refresh.completed`：刷新终态，含 success/not-modified/error code、耗时和脱敏诊断。

同步 `before/fetched/entry.process` 受严格 timeout、fuel、内存、输出限制，且不可获得 AI/MCP capability；首版官方插件不订阅。`persisted/completed` 从 outbox 至少一次异步投递；官方 AI 插件只订阅 `feed.refresh.persisted` 并返回 job intents，不在 event callback 内读取数据库或调用模型。

### 16.3 顺序、失败与幂等

- 每个 event、per-user delivery、job 和 artifact 分别有稳定 idempotency key/唯一约束；重试保留相同 event key。
- 生命周期默认 `FAIL_OPEN`，记录错误并继续 RSS；首版没有可选择 fail-closed 的安全过滤插件。
- 官方插件自动 job key 固定包含 event、plugin、user、entry、operation 和 config hash，outbox 重投不能重复收费或生成重复 artifact。
- plugin host KV 有每插件/用户容量和写速限制，不允许成为数据库代理；官方 AI 插件 v1 manifest 不申请 KV。
- 连续 guest failure 触发 circuit breaker/quarantine；execution UI 只显示脱敏错误和合法 retry。

### 16.4 Manifest 与权限

Manifest v1 至少包含 `pluginKey`、semver、WIT ABI、operations、lifecycle subscriptions、config/artifact schema、requested capabilities、ambient permission 空集、component digest 和官方签名。`raindrop.ai-content` 必须申请 `ai.generate_structured`，可按配置申请 `mcp.call_tool`。

requested capability 不等于授权。host 按 invocation 绑定用户、operation、provider 或 MCP connection/exact tool，并强制 fuel、64 MiB memory、epoch timeout、WIT/artifact output、token、cost 和 concurrency ceiling。插件拿不到 secret、任意 HTTPS、文件、环境、进程、数据库、真实 transport 或 raw connection。

Manifest、WIT、event、config 和 artifact 各自版本化；v1 record 只 additive 增加可选字段，不兼容变更使用新 major。新 WIT major 发布后旧 major 至少保留两个稳定版本并有明确 deprecation warning。

## 17. MCP 双向集成

### 17.1 Raindrop 作为 MCP client

管理员或用户可配置外部 MCP server，transport 首版支持 Streamable HTTP 和受限 stdio child process。连接配置包含凭据、exact tool allowlist、每工具 scope/effect、超时、并发和审批策略。

- AI 插件不实现 transport，只能调用 MCP client domain service 暴露的 host capability `mcp.call_tool`；credential、endpoint secret、stdio command 和 raw connection 不过 ABI。
- 摘要/翻译默认不使用 MCP。用户为特定 operation 开启 context enrichment 后，有效授权是用户 + plugin config + operation + connection + exact tool allowlist 的交集。
- `feed.refresh.persisted` 自动 job 最多调用 2 个明确标记 `READ_ONLY` 的工具；首版没有逐次确认 UI，因此有副作用或 effect unknown 的工具在自动和手动 AI 流程中都返回 `MCP_SIDE_EFFECT_CONFIRMATION_REQUIRED`。
- host 校验 tool input/output schema，并限制 arguments/result 大小、单次超时、并发、最大 tool-call 次数、递归深度、token 和 cost；tool description/result 都是不可信数据。
- MCP 失败按 operation 配置 fail-open 回退无工具生成，或 fail-closed 结束 artifact/job；都不能阻塞 RSS 入库或原文阅读。
- 完整调用审计保存用户作用域的 bounded/redacted arguments/result、digest、schema、duration 和 error code；日志不复制秘密或完整 result。
- stdio server 使用受限工作目录、清理后的环境变量和命令 allowlist；Web 用户不能提供任意 shell command。

### 17.2 Raindrop 作为 MCP server

服务入口：

- `/mcp`：Streamable HTTP，适合远程或同机 AI agent。
- `raindrop mcp --stdio`：本地 agent 子进程模式，连接同一配置/数据库。

首版 resources：

```text
raindrop://entries/{entryId}
raindrop://feeds/{feedId}
raindrop://categories
raindrop://artifacts/{artifactId}
```

首版 tools：

```text
list_entries, get_entry, search_entries
mark_entry_read, star_entry
list_subscriptions, subscribe_feed
summarize_entry, translate_entry
list_content_artifacts
```

工具返回稳定 JSON Schema，并复用 `/api/v1` 领域服务，不直接访问数据库。每个调用由 hashed personal token 或配置的 OAuth 流程认证，并映射 scopes：`entries:read`、`entries:write`、`subscriptions:read`、`subscriptions:write`、`ai:invoke`、`admin`。

MCP 请求不使用浏览器 Cookie 或 CSRF 机制。所有结果按 token 所属用户过滤；管理员 scope 不隐式授予跨用户内容读取，跨用户操作必须使用单独的显式 admin tool。

`summarize_entry` 和 `translate_entry` 复用 content orchestrator，并创建/继承 `callChainId`、instance identity、visited target/tool 和 depth budget。AI-plugin outbound MCP call 默认 depth 2；同一 chain 重复 `(serverIdentity, toolName)`、depth 0，或目标为当前 Raindrop 且会再次进入 summarize/translate 时返回 `MCP_RECURSION_BLOCKED`，防止 MCP server → AI plugin → MCP client 的同链自调用。

### 17.3 MCP 协议兼容性

实现使用 rmcp 并锁定经过集成测试的 MCP protocol revision。CI 对 Streamable HTTP 初始化、tools/list、resources/read、tool call、取消、错误和 stdio framing 运行契约测试。协议扩展必须是 additive；未知 client capability 被忽略而不是导致崩溃。

## 18. GitHub Actions 与发布

### `ci.yml`

- Rust：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、SQLite 测试。
- Web：`npm ci --ignore-scripts` 后仅批准所需构建脚本，运行 lint、typecheck、Vitest、build。
- PostgreSQL/MySQL 服务容器运行迁移与仓储契约测试。
- 生成 OpenAPI 并检查前端类型无漂移。

### `release-binaries.yml`

Tag `v*` 或手工触发：

- Linux amd64/arm64（优先静态 musl）。
- Windows amd64。
- macOS amd64/arm64。
- 产物包含 Web UI、LICENSE、README 和示例配置；发布 SHA-256 checksums。

### `docker.yml`

参考提供的 Owl workflow：QEMU + Buildx，发布 `linux/amd64,linux/arm64` 到 GHCR；当 Docker Hub secrets 存在时同时发布 `czyt/raindrop`。使用 semver、tag 和 `latest` metadata，启用 GHA layer cache。

Dockerfile 为 Node 前端构建 → Rust 构建 → 最小运行镜像的多阶段构建。运行用户非 root，数据目录 `/data`，暴露健康检查，镜像中不包含 npm、Cargo 缓存或编译工具链。

## 19. 命令

```bash
# 开发
cargo run
npm --prefix web run dev

# 后端验证
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features

# 前端验证
npm --prefix web ci --ignore-scripts
npm --prefix web run lint
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build

# 生产构建
npm --prefix web run build
cargo build --release --locked

# 容器
docker build -t raindrop:dev .
docker run --rm -p 8080:8080 -v raindrop-data:/data raindrop:dev
```

## 20. 代码风格

Rust 领域错误使用明确枚举并保留 source，不用字符串判断控制流程：

```rust
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    #[error("feed URL is invalid")]
    InvalidUrl,
    #[error("feed already subscribed")]
    AlreadySubscribed,
    #[error("feed fetch failed")]
    Fetch(#[source] FeedFetchError),
}
```

规则：

- 文件和函数 `snake_case`，类型 `UpperCamelCase`，常量 `SCREAMING_SNAKE_CASE`。
- 禁止在 route handler 写 SQL 或 Feed 解析逻辑。
- 禁止 `unwrap`/`expect` 处理外部输入和生产启动路径；测试可在意图明确时使用。
- 公共 DTO 与数据库实体分离。
- 前端组件保持单一职责；业务请求集中在 typed client/query 层。
- 前端控件先查 ASTRYX 0.1.6 文档；业务代码按 feature 拆分，单文件默认约 250 行以内。

## 21. 测试策略

- 单元：配置优先级、URL/IP 安全分类、Feed/条目标识、刷新间隔、状态游标、OIDC claim 映射。
- 仓储契约：同一测试套件运行 SQLite、PostgreSQL、MySQL，验证唯一约束、事务、分页和稀疏阅读状态。
- API 集成：setup 状态机、登录/退出、CSRF、用户隔离、管理员授权、统一错误。
- 抓取集成：本地测试服务器覆盖 304、重定向、超时、私网拒绝、压缩上限、恶意 HTML。
- AI：provider mock、`ai.generate_structured` secret 隔离、结构化输出拒绝、artifact cache/invalidation、token/cost 配额、有界重试和 prompt injection 防护。
- 插件：bundled component digest/signature、官方组件不走 native shortcut、manifest/WIT/config/artifact 合同、五个 lifecycle fixture、无 ambient 权限、fuel/内存/超时/输出、outbox 重试和幂等。
- 事务边界：provider/MCP mock 证明 Feed 抓取与持久化事务内调用数为 0；rollback 不投递，commit 后才 fan-out/job。
- MCP：AI plugin 默认关闭、用户 + operation + connection + exact tool allowlist、自动只读、副作用拒绝、恶意 tool 描述/响应、schema/大小/超时/并发/call/depth/token/cost、fail-open/fail-closed、脱敏审计和递归环路。
- 插件 API/UI：只返回官方插件及查询/启停/配置/execution/retry；install/catalog route 与 marketplace/编排 UI 不存在。
- 前端：组件和状态测试；每个 locale 至少渲染主要页面。
- ASTRYX：运行已使用组件的 docs/API 检查、axe 键盘/ARIA 测试和 production bundle smoke，防止开发构建可用而生产崩溃。
- E2E：无配置首次设置、本地登录、OIDC 模拟登录、订阅、OPML 导入、阅读/收藏、主题切换、管理员禁用用户。
- 发布：运行 binary `--version`、嵌入 UI smoke、Docker 健康检查和非 root 检查。

## 22. 工程边界

### 始终执行

- 外部输入在边界验证，数据库查询参数化。
- 每个用户资源查询包含用户作用域。
- schema 变更同时更新迁移、三数据库契约测试和备份文档。
- 提交前运行与改动风险匹配的测试。
- 日志对密码、token、cookie、数据库密码和 OIDC secret 脱敏。

### 仅在目标明确要求时执行

- 引入 Redis、消息队列、搜索集群或第二事实源。
- 支持开放互联网匿名注册。
- 支持任意自定义 JavaScript。
- 直接导入 CommaFeed 数据库。
- 允许自动 Feed 工作流在没有明确 allowlist 和预算时调用 MCP 工具。
- 开放第三方插件安装、签名根、catalog、marketplace 或多插件编排。

### 绝不执行

- 提交秘密、真实账号或生产数据库 URL。
- 在浏览器 localStorage 保存认证 token。
- 未清洗就渲染 Feed HTML。
- 为通过测试删除安全检查或失败测试。
- 依赖数据库默认隔离级别来替代唯一约束和幂等设计。
- 让第三方插件获得 ambient 文件系统、环境变量、任意网络或进程权限。
- 为官方 AI 插件添加绕过 WIT/capability host 的 native processor 或 route handler 模型调用。
- 把 provider secret、MCP credential/transport/raw connection 传过插件 ABI。
- 把模型输出直接传给 DOM、SQL、shell、路径或插件权限配置。

## 23. 分阶段交付

1. 工程基础与 bootstrap：Rust/React 构建、配置、SQLite 迁移、设置向导、管理员、本地登录、嵌入 UI。
2. RSS 纵向链路：订阅、调度抓取、解析清洗、文章列表、阅读/收藏。
3. 多用户组织：分类、未读游标、用户设置、注册策略、管理员。
4. OIDC：provider、绑定、claim 策略、管理界面。
5. 官方 AI 内容插件：provider core、job/artifact、版本化 WIT/manifest/config、bundled signed `raindrop.ai-content`、手动摘要/翻译和 Reader sidecar。
6. 插件 host 与自动生命周期：五个 event contract fixture、Wasmtime capability broker、`feed.refresh.persisted` fan-out、execution/retry UI；不开放安装或 marketplace。
7. MCP：外部 server 连接、`mcp.call_tool` broker、只读 enrichment/failure/递归策略，以及 Raindrop Streamable HTTP/stdio server。
8. 可移植性：PostgreSQL/MySQL 契约、OPML、设置 JSON、保留/备份。
9. 产品化：响应式三栏、主题、多语言、键盘体验、可访问性。
10. 发布：CI、binary、Docker、文档、端到端发布验证。

每一阶段都必须产生可运行软件；后续阶段不能用占位实现冒充完成。

## 24. 完成标准

- 空目录运行时可通过带一次性 token 的设置向导完成 SQLite 初始化并登录管理员。
- 全环境变量部署无需交互即可初始化；重启不重复创建管理员。
- 同一二进制可连接 SQLite、PostgreSQL、MySQL，并通过同一仓储契约测试。
- 本地账号和至少一个标准 OIDC provider 端到端登录成功。
- 两个用户订阅同一 Feed 时只保存一份 Feed 和文章，状态严格隔离。
- 重复或并发抓取不会生成重复条目；mark-all 不逐篇写状态。
- Reader 在桌面三栏、中屏两区和移动单任务路由下保持同一来源/过滤/选择/已读/收藏状态；浏览器返回可恢复列表与正文位置。
- `J/K`、`N/P`、`M`、`S`、批量已读快照、下一未读来源和“载入新文章”通过集成测试；迟到请求不能覆盖更新的来源，乐观写失败会回滚树、列表和正文。
- OPML 与设置导入/导出通过限制、预览、回滚和秘密排除测试。
- 发行二进制只交付官方签名 `raindrop.ai-content` Wasm Component；它通过通用 WIT/manifest/capability host 提供摘要和翻译，结果缓存、版本化、可失效且不覆盖原文，不存在 native shortcut。
- 五个 Feed lifecycle event 的 v1 fixture 和 host contract 通过测试；官方插件只消费 `feed.refresh.persisted`，outbox 重投只产生一个 user/entry/operation/config job 和 artifact。
- Feed transaction 内 provider/MCP 调用数为 0；插件、provider 或 MCP 失败不改变 RSS 入库、原文阅读、已读或收藏。
- Raindrop AI 插件可按用户 + operation + connection + exact tool allowlist 调用测试 MCP server 的只读工具，默认关闭；自动只读、副作用拒绝、fail-open/fail-closed、审计和递归阻断通过测试。
- Raindrop 自身可通过 Streamable HTTP 和 stdio 暴露用户作用域 RSS/AI 工具；`summarize_entry`/`translate_entry` 复用 content orchestrator 并传播 call-chain/depth。
- 首版 OpenAPI/UI 只有官方插件的查询、启停、配置、execution 和 retry，不存在 install/catalog/marketplace/多插件编排入口。
- React UI 来自二进制内嵌资源，生产运行不需要 Node/npm 或独立静态服务器。
- 中文、英文及三种主题模式完成主要流程；键盘和 reduced-motion 可用。
- CI 验证格式、lint、单元、三数据库、前端和构建；tag 可生成 binary 与双架构 Docker 镜像。
- 安全测试证明 setup 抢占、IDOR、CSRF、OIDC 重放、Feed SSRF 和存储型 XSS 防护有效。
- AI/MCP/插件安全测试证明 Feed、模型与 tool 内容不能扩大权限，secret/transport 不过 ABI，插件无 ambient 权限，MCP token 不能跨用户读取。

## 25. 内部自审结果

- 占位符扫描：无占位标记，所有决策已绑定。
- 一致性：多用户、OIDC、三数据库、嵌入 UI、导入、主题、多语言、唯一官方 AI 插件、版本化扩展合同、双向 MCP 和发布目标均有架构与完成标准。
- DDIA：记录系统、派生 artifact、幂等边界、事务 outbox、至少一次投递、写放大、索引、故障恢复和 schema 演进均有明确结论。
- 安全：首次设置、认证、跨租户、外部 URL、导入文件、prompt injection、官方 Wasm sandbox、AI/MCP broker、递归和 scope 均有滥用场景与控制。
- 范围：总目标拆为十个可运行阶段；首版不声称开放第三方插件生态、市场、远程安装或多插件编排。
