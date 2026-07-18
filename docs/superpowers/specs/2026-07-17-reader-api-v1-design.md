# Raindrop Reader API v1 设计

日期：2026-07-17

状态：bounded 内部评审通过（Critical 0 / Important 0）

关联规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`

## 1. 决策摘要

首个 Reader API 垂直切片只交付三个端点：

```text
GET   /api/v1/entries
GET   /api/v1/entries/:id
PATCH /api/v1/entries/:id/state
```

它直接复用已经完成的用户作用域列表、详情、游标快照和 sanitized content 能力，并补齐单篇已读/收藏原子写入。这个范围足以让下一阶段 ASTRYX Reader 真实展示 All、Unread、Starred、单 Feed、正文深链接以及 `M`/`S` 状态动作。

本切片不加入分类、订阅树、Feed 刷新、批量已读、搜索、SSE、OPML、AI 或 MCP。`POST /entries/mark-read` 必须同时定义稳定 ingestion snapshot、per-subscription frontier 和 explicit unread override 的事务语义，作为紧随其后的独立切片实现，不能用 live Feed head 代替。

用户已明确把后续设计确认委托给内部评审，因此本文件的 Critical/Important 内部评审结论等价于本切片批准，不再设置逐项人工确认门。

## 2. 方案比较

| 方案 | 内容 | 收益 | 风险 | 结论 |
| --- | --- | --- | --- | --- |
| A. 只读 | 列表与详情 | 代码最少 | Reader 无法持久化核心 `M`/`S` 动作 | 不采用 |
| B. 真实 Reader 最小闭环 | 列表、详情、单篇状态 | 最小但完整的阅读交互切片；无迁移 | 需要一个跨数据库原子写入合同 | **采用** |
| C. 同时加入订阅与批量已读 | 七个端点 | 一次覆盖更多 Reader 能力 | 网络服务接线、Feed 树分页、snapshot frontier 和并发语义相互耦合 | 拆到后续切片 |

## 3. Objective

为已登录用户提供稳定、不可跨租户访问的 Reader 数据接口：

- 按 `ALL | UNREAD | STARRED` 和可选 Feed 游标分页读取条目。
- 通过条目深链接读取持久化的安全正文、惰性图片元数据和 enclosure。
- 原子更新单篇 `isRead` 与 `isStarred`，只改变请求中出现的字段。
- 所有响应使用统一错误 envelope 和敏感响应缓存策略。

完成后，Reader 前端不需要读取数据库实体、不需要自己解析 cursor，也不需要伪造本地状态持久化。

## 4. Tech Stack

- Rust edition 2024，MSRV 1.94。
- Axum 0.8.9 路由与 extractor。
- SeaORM 1.1.19，支持 SQLite、PostgreSQL、MySQL。
- Serde 1.0.228；HTTP 字段 camelCase，枚举 UPPER_SNAKE_CASE。
- 现有 `CurrentUser`、`CsrfGuard`、`ApiJson`、`ApiError`、`FeedRepository`。
- 不新增 crate，不新增数据库迁移。

## 5. Commands

```bash
# 格式
cargo fmt --all -- --check

# 本切片聚焦测试
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features --test feed_state_contracts

# 相关回归
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test feed_query_contracts
cargo test --locked --all-features --test session_security
cargo test --locked --all-features --test setup_auth_api

# 完整验证
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
git diff --check
```

PostgreSQL/MySQL 合同继续使用现有环境变量与串行 CI service；CI 对 `feed_state_contracts postgres` 和 `feed_state_contracts mysql` 分别使用 `--test-threads=1`，避免 rollback/remigrate 共用后端时交叉破坏。本地没有 URL 时明确跳过，SQLite 必须本地执行。MySQL 的确定性锁等待观察使用同一数据库账号可见的 `SHOW PROCESSLIST` 并按 T2 connection ID 匹配等待状态，不要求全局 `PROCESS` 权限，也不查询 `information_schema.innodb_trx`。

## 6. Project Structure

```text
src/api/entries.rs              Reader HTTP route、wire DTO、mapping、错误映射
src/api/error.rs                Reader method-not-allowed 统一错误
src/api/mod.rs                  注册 entries 模块
src/api/routes.rs               合并 Reader router；复用 no-store middleware
src/feeds/state.rs              单篇状态领域类型与跨数据库事务
src/feeds/dto.rs                EntryStateDto / UpdateEntryState
src/feeds/mod.rs                导出 Reader 状态领域接口
src/feeds/query.rs              扩展 redacted RepositoryError
tests/reader_api.rs             HTTP seam：认证、授权、DTO、错误、缓存、CSRF
tests/feed_state_contracts.rs   repository seam：稀疏状态、幂等、并发、三后端
```

`src/api/routes.rs` 不接收 Reader handler 逻辑。领域 DTO 不添加 Serde wire derive；HTTP 层显式映射，避免数据库/领域结构成为公开合同。

## 7. HTTP Contract

### 7.1 `GET /api/v1/entries`

认证：`CurrentUser`。读取接口不要求 CSRF。

Query：

| 字段 | 默认 | 约束 |
| --- | --- | --- |
| `cursor` | absent | 上一页返回的 canonical opaque cursor，最多 1,024 bytes |
| `limit` | `50` | `1..=100` |
| `feedId` | absent | canonical lowercase hyphenated UUID；未订阅 ID 返回空页 |
| `state` | `UNREAD` | 精确 `ALL | UNREAD | STARRED` |

query struct 使用 `deny_unknown_fields`。Axum query rejection、重复字段、错误类型、未知字段、非法 limit/UUID/cursor 都映射为统一 422。

`/api/v1/entries` 使用 scoped nested router，并在 inner router 上设置 fallback 和 method-not-allowed fallback。Axum 0.8.9 的 nest prefix 不覆盖尾随斜线 `/api/v1/entries/`，因此 outer Reader router 还要显式注册该路径为 JSON 404。未知子路径返回统一 404 JSON；已知路径的错误 method 返回统一 405 JSON；outer Reader router 的 no-store middleware 覆盖 exact、trailing slash、nested fallback 和 method fallback。禁止用与 `/{entry_id}` 冲突的同级 catch-all route。

响应：

```json
{
  "items": [
    {
      "entryId": "00000000-0000-4000-8000-000000000301",
      "feedId": "00000000-0000-4000-8000-000000000101",
      "feedTitle": "Example Feed",
      "siteUrl": "https://example.com/",
      "title": "Example entry",
      "author": "Example author",
      "summary": "Summary",
      "canonicalUrl": "https://example.com/articles/2",
      "publishedAtUs": 1784246400000000,
      "sortAtUs": 1784246400000000,
      "isRead": false,
      "isStarred": false
    }
  ],
  "nextCursor": null,
  "snapshotGeneration": 42
}
```

`snapshotGeneration` 只冻结 ingestion membership，不冻结之后发生的用户 read/star 状态。

### 7.2 `GET /api/v1/entries/:id`

认证：`CurrentUser`。`:id` 必须是 canonical UUID。

不存在、订阅前不可见、未订阅或属于其他用户的条目都返回相同 404，不暴露存在性。

响应包含列表字段，另加：

```json
{
  "contentHtml": "<p>Safe content</p>",
  "inertImages": [],
  "enclosures": []
}
```

`contentHtml` 只能来自持久化且成功解码的 sanitized-content envelope。公开 API 中 `inertImages` 和 `enclosures` 始终是数组，缺失 enclosure envelope 映射为空数组。

### 7.3 `PATCH /api/v1/entries/:id/state`

认证顺序：`CurrentUser`、`CsrfGuard`、`ApiJson`。这样同一请求复用已解析 session，且 body 最后提取。

请求：

```json
{
  "isRead": true,
  "isStarred": false
}
```

规则：

- 至少出现一个字段。
- 只改变出现的字段，另一个字段在同一事务中保留。
- unknown field、显式 `null`、非 boolean、空对象都返回 422。
- 重复相同请求幂等，不刷新 `starred_at`，不增加内部 revision。
- 同字段并发按数据库提交顺序 last-write-wins；不同字段并发必须合并而不是互相覆盖。
- v1 不暴露数据库 revision、ETag 或 `If-Match`。未来若需要显式多设备冲突，必须使用覆盖 `entry_states.revision + subscriptions.state_revision` 的 opaque validator，不能直接冻结单表 revision 到 wire contract。

响应：

```json
{
  "entryId": "00000000-0000-4000-8000-000000000301",
  "isRead": true,
  "isStarred": false
}
```

## 8. Repository Contract

新增公开领域接口：

```rust
pub struct UpdateEntryState {
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

pub struct EntryStateDto {
    pub entry_id: String,
    pub is_read: bool,
    pub is_starred: bool,
}

impl FeedRepository {
    pub async fn update_state_for_user(
        &self,
        user_id: &str,
        entry_id: &str,
        patch: UpdateEntryState,
    ) -> Result<Option<EntryStateDto>, RepositoryError>;
}
```

`None` 表示条目不存在或对用户不可见。`RepositoryError::InvalidStatePatch` 表示空内部 patch。

## 9. Transaction and DDIA Review

关系数据库仍是记录系统；`read_override` 和 `is_starred` 是用户作用域状态，未读列表是可重建派生查询。

单篇写入必须在一个短事务中完成：

1. 校验 canonical user/entry UUID 和非空 patch。
2. 在事务内通过 `subscriptions JOIN entries` 授权，并要求 `entry.feed_sequence > subscription.start_sequence`。
3. 第一条数据库步骤只定位、授权并序列化对应 Subscription：PostgreSQL 使用 `FOR UPDATE OF s`，MySQL 使用 `FOR UPDATE`；SQLite 的第一条事务语句使用 scoped no-op `UPDATE subscriptions SET state_revision = state_revision ...` 获取 writer lock。
4. 锁返回后使用第二条语句重新读取当前 subscription frontier、entry 和 `entry_states`。PostgreSQL 由第二个 statement 获得 post-wait READ COMMITTED snapshot；MySQL 第二条使用 locking/current read；SQLite 已持 writer lock。禁止在同一条等待锁的 PostgreSQL join 中读取要保留的 `entry_states` 字段。
5. 读取 base read：`entry.feed_sequence <= subscription.read_through_sequence`。
6. 请求 read 等于 base 时保存 `read_override = NULL`；否则保存显式 boolean。
7. 星标从 false→true 时使用数据库时钟设置 `starred_at`；重复 true 保留时间；false 清空时间。
8. 首次有语义的状态写入 revision=1；后续仅在持久化语义改变时 revision+1。
9. 若 next state 为 `read_override IS NULL AND is_starred = FALSE`，existing row 必须使用 `(user_id, entry_id, revision)` guard 删除；无 existing row 时不写。这样恢复稀疏状态，不保留 neutral tombstone。
10. 在锁定后的第二次读取中保留未提供字段，避免 read PATCH 覆盖并发 star PATCH。
11. 返回 canonical effective state 后提交。网络、解析、AI、MCP 均不进入该事务。

这不是分布式事务，也不声明 end-to-end exactly-once。事务只保证单数据库内的授权、状态合并和 revision 原子性。

## 10. Error Contract

| 条件 | HTTP | code |
| --- | ---: | --- |
| JSON/query/path/limit/cursor/state/patch 验证失败 | 422 | `VALIDATION_ERROR` |
| session 缺失、失效、过期、用户禁用 | 401 | `AUTHENTICATION_REQUIRED` |
| CSRF/Origin/Host 校验失败 | 403 | `FORBIDDEN` |
| well-formed entry 不存在或不可见 | 404 | `NOT_FOUND` |
| Reader 已知路径使用不支持的 method | 405 | `METHOD_NOT_ALLOWED` |
| DB、损坏数据、sanitized envelope 解码失败 | 500 | `INTERNAL_ERROR` |

错误正文继续使用现有 envelope，不包含 SQL、backend error、正文、cursor payload、Cookie、CSRF、文件路径或栈。

Reader router 的成功和失败响应均添加：

```http
Cache-Control: no-store
Pragma: no-cache
```

## 11. Threat Model

信任边界：HTTP query/path/body、session/CSRF header、数据库持久化 Feed 内容。

- Spoofing：所有端点用 `CurrentUser`；user ID 不来自请求参数。
- Tampering：参数化 SQL；PATCH body 严格 boolean/known fields。
- Repudiation：本切片不新增内容日志；允许记录 route template、request ID、status、typed error，不记录正文/secret。
- Information disclosure：所有读取与写入都通过 Subscription join；猜测 UUID 不授权。
- Denial of service：沿用 64 KiB body limit、limit<=100、cursor<=1,024；无外部网络调用。
- Elevation of privilege：普通用户没有管理员 bypass；CSRF mutation 无 route-specific Origin/CORS 例外。

## 12. Code Style

边界类型与 wire 类型分开，handler 只做验证、映射和调用：

```rust
async fn get_entry(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(entry_id): Path<String>,
) -> Result<Json<EntryDetailResponse>, ApiError> {
    let repository = repository(&state)?;
    let detail = repository
        .get_detail_for_user(&user.id, &entry_id)
        .await
        .map_err(map_repository_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(detail.into()))
}
```

禁止把 repository error 的 source 文本放进 HTTP body；禁止在 route 中拼接 SQL；禁止为本切片创建通用万能 service/provider 抽象。

## 13. Testing Strategy and Confirmed Seams

本切片预先确认两个测试 seam：

1. HTTP seam：通过 `build_router` 发真实 cookie/query/body 请求，观察 status、header、JSON；不直接调用 handler。
2. Repository seam：通过 `FeedRepository::update_state_for_user` 观察公开返回，并只在验证稀疏存储/revision/时间不变量时读取数据库行。

TDD 采用垂直 red→green：先列表/详情 HTTP，再 repository 状态，再 PATCH HTTP。不会先批量写完所有测试。

关键用例：

- 默认 UNREAD、ALL/STARRED、Feed filter、cursor 链和跨用户/filter cursor 拒绝。
- sanitized detail 映射、数组稳定性、猜测/跨租户统一 404。
- read-only、star-only、组合 patch、explicit unread、base-state null override、清除最后状态后删除 neutral row。
- false→true starred_at、重复 true 不变、true→false 清空。
- 空 patch/null/type/unknown field、认证、CSRF、Origin、缓存 header。
- SQLite 两连接并发不同字段最终同时存在；PostgreSQL/MySQL 使用确定性 barrier：T1 持有 Subscription lock、T2 已开始等待、T1 提交后 T2 必须看到并保留 T1 字段。
- CI 分别串行执行 PostgreSQL/MySQL state contract；MySQL observer 不依赖超出普通测试账号的全局权限。
- TDD cycle 固定为 list tracer RED→GREEN、detail/isolation RED→GREEN、逐类 validation RED→GREEN；PATCH 同样先成功 tracer，再逐类 body/auth/CSRF/not-found/cache。
- 相关 RSS 查询、session、setup 回归和 full suite。

## 14. Boundaries

### Always

- Rust edition 2024、MSRV 1.94、Cargo `--locked`。
- 所有用户数据查询显式接收 authenticated user ID。
- 外部输入只在 API boundary 验证；SQL 参数化。
- mutation 要求 `CurrentUser + CsrfGuard`。
- TDD red→green；完成声明前 fresh full verification。

### Internally review before changing

- 数据库迁移、依赖、公开 endpoint/字段、认证/CSRF 逻辑、缓存策略。
- 仅 Critical/Important 阻塞；一轮修复后 bounded re-review。

### Never

- 暴露 DB entity、SQL/backend error、raw Feed body 或未经验证 HTML。
- 通过 entry ID 本身授权。
- 在状态事务中执行网络、AI、MCP 或插件调用。
- 修改用户原有未跟踪 `.superpowers/research/` 与 `node_modules/`。

## 15. Success Criteria

- 三个端点按本合同工作，wire 字段与错误稳定。
- 所有成功/失败、Reader scoped 404 和 method-not-allowed 响应 no-store/no-cache，且不会返回 embedded HTML。
- 跨租户读取和写入不可区分地失败，不产生 state row。
- PATCH 对 omitted field 原子保留，重复请求幂等，neutral row 被删除，SQLite 与 env-gated PostgreSQL/MySQL 确定性并发合同通过。
- 不新增 schema/dependency，不改变已有 Feed ingestion 行为。
- focused tests、相关回归、clippy、full suite、`git diff --check` 全部 fresh 通过。
- bounded review 为 Critical 0 / Important 0 后提交并推送。

## 16. Open Questions

无阻塞问题。批量已读、订阅列表/创建/刷新和 Reader UI 已明确排入后续独立切片。
