# Raindrop 分类组织 v1 设计

日期：2026-07-18

状态：内部批准，可直接实施

关联规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`

## 1. 目标与假设

本切片交付 CommaFeed 式的一层分类组织：用户可创建、重命名、排序和删除分类，把自己的订阅移动到分类或未分类，并通过分类路由阅读条目。分类必须进入现有 Reader 的同一状态、路由和 OpenAPI 生成链，不创建第二套前端 store 或手写 wire DTO。

内部采用以下假设：

1. v1 分类只有一层，不支持父子分类、标签树或任意嵌套。
2. 删除分类不会删除订阅、Feed、条目或阅读状态；相关订阅原子回到未分类。
3. 分类标题对同一用户按 `trim + Unicode lowercase` 后唯一，保留用户输入的展示大小写。
4. 每用户最多 250 个分类，因此分类列表一次返回全部项目，不使用分页。
5. 分类顺序以非负 `position` 加 UUID `id` 打破平局；并发相同 position 是合法状态，不覆盖、不丢失数据。
6. 分类路由默认展示未读条目，Reader 的 All/Unread/Starred 和单 Feed 语义保持不变。

## 2. 技术栈与命令

- Rust edition 2024、Axum、SeaORM/SeaORM Migration、SQLite/PostgreSQL/MySQL。
- React 19、TypeScript、React Router、ASTRYX 0.1.6、Lingui、Vitest、Playwright。
- 数据库验证：`cargo test --locked --all-features --test organization_migrations --test category_repository`。
- API 验证：`cargo test --locked --all-features --test category_api --test organization_openapi_contract --test subscription_api --test reader_api`。
- 前端验证：`cd web && npm run check:reader-types && npm run typecheck && npm run test:ci && npm run build`。
- 浏览器验证：`cd web && PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:e2e`。
- 最终 Rust 门禁：`cargo test --locked --all-features`。

## 3. 项目结构与模块边界

```text
src/db/migration/organization.rs       分类表与 subscriptions.category_id
src/db/entities/category.rs            SeaORM 分类实体
src/organization/category.rs           分类校验、仓储、事务和 DTO
src/api/categories.rs                  当前用户分类 HTTP API
src/feeds/subscription.rs               订阅标题/位置/分类 patch
src/feeds/query.rs                      categoryId 条目过滤与 cursor binding
docs/openapi/organization-v1.json       分类 API 的权威 wire contract
docs/openapi/subscription-v1.json       additive 订阅 patch/category 字段
docs/openapi/reader-v1.json             additive categoryId 查询参数
web/src/features/reader/categories/     分类 API、Dialog 与 UI helpers
web/src/features/reader/model/          单一 Reader controller/reducer 扩展
web/src/features/reader/routes/         分类深链接
```

`organization` 拥有分类的创建、列表、更新和删除。`feeds` 仍拥有 subscription 行及其 patch；它只能把订阅指向同一用户可见的分类。Reader 查询继续由 `FeedRepository` 执行，并把 user/category 共同绑定到游标过滤哈希。

## 4. 数据模型与 DDIA 内部评审

### 4.1 表与索引

```text
categories(
  id VARCHAR(36) PRIMARY KEY,
  user_id VARCHAR(36) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title VARCHAR(200) NOT NULL,
  normalized_title VARCHAR(320) NOT NULL,
  position BIGINT NOT NULL,
  created_at operational_timestamp NOT NULL,
  updated_at operational_timestamp NOT NULL
)

UNIQUE categories(user_id, normalized_title)
INDEX categories(user_id, position, id)

subscriptions.category_id VARCHAR(36) NULL
FOREIGN KEY subscriptions(category_id) REFERENCES categories(id) ON DELETE SET NULL
INDEX subscriptions(user_id, category_id, position, id)
```

### 4.2 一致性与并发

- `users` 行是每用户分类配额、标题唯一预检查和默认 position 分配的串行化点。SQLite 使用空更新取得 writer lock，PostgreSQL/MySQL 使用 `SELECT ... FOR UPDATE`。
- 唯一 `(user_id, normalized_title)` 是最终重复仲裁；应用预检查只用于稳定返回 `409 CONFLICT`。
- 创建时默认 position 为当前最大值加 1024；显式 patch 可写任意非负 i64。相同 position 通过 id 稳定排序，不需要分布式序号分配。
- 删除依赖 `ON DELETE SET NULL`，分类和订阅关系在同一数据库提交中改变。条目、Feed 与稀疏阅读状态不受影响。
- categories 是记录系统；TreeList 分组和分类未读数由 subscriptions/entries 派生，不另建计数事实表。
- 首版不预建全文索引、嵌套集合、闭包表或 materialized category counters；真实读取只有每用户最多 250 分类和最多 1000 订阅。

### 4.3 演进

- 本迁移只新增表、列、外键和索引，符合 expand-first。
- `categoryId` 是 additive、nullable 字段；旧客户端继续把所有订阅视为平铺列表。
- 未来嵌套分类必须新增可空 `parent_id` 和独立循环检测合同，不能重新解释当前 `position`。

## 5. 输入、API 与错误合同

### 5.1 分类 API

```text
GET    /api/v1/categories
POST   /api/v1/categories
PATCH  /api/v1/categories/:categoryId
DELETE /api/v1/categories/:categoryId
```

分类 DTO：

```json
{
  "categoryId": "uuid",
  "title": "Technology",
  "position": 1024
}
```

- 创建 body：`{ "title": string }`。
- patch body 至少包含 `title` 或 `position`；未知字段、空 patch、负 position 返回 `422 VALIDATION_ERROR`。
- 标题 trim 后为 1–80 个 Unicode scalar，UTF-8 不超过 200 bytes，不能含 C0/C1 control；normalized UTF-8 不超过 320 bytes。
- 标题冲突返回 `409 CONFLICT`；超过 250 返回 `409 CATEGORY_LIMIT_REACHED`。
- 不存在或属于其他用户的 category ID 都返回 `404 NOT_FOUND`。
- mutation 需要 session、CSRF、Origin/Host 校验与用户级 mutation limiter。

### 5.2 订阅 patch

`PATCH /api/v1/subscriptions/:subscriptionId` 接受：

```json
{
  "categoryId": "uuid or null",
  "titleOverride": "string or null",
  "position": 2048
}
```

字段均可省略，但至少一个必须出现。`categoryId = null` 移到未分类。非空 category 必须属于当前用户，否则统一 `404 NOT_FOUND`。`titleOverride` trim 后为空等价于 null，最长 200 UTF-8 bytes；position 必须非负。

Subscription DTO additive 增加 `categoryId`、`titleOverride`、`position`，现有 `title` 继续是 `titleOverride ?? feed.title ?? normalizedUrl` 的有效展示标题。

### 5.3 Reader category filter

`GET /api/v1/entries` additive 接受 `categoryId`。`feedId` 与 `categoryId` 互斥，同时提供返回 `422`。查询必须同时匹配 `subscriptions.user_id` 与 `subscriptions.category_id`；不存在或跨用户 category 得到空页，不暴露资源存在性。cursor filter hash 包含 user、state、feedId 和 categoryId，不能跨来源重放。

## 6. Reader UI/UX 合同

- Source Tree 顺序：Unread、All、Starred、分类分支、Uncategorized 分支。
- 分类分支只包含 `subscription.categoryId` 相同的 Feed；没有分类的 Feed 放入 Uncategorized。空分类仍显示，便于用户选择和管理。
- 分类节点显示其子 Feed 未读数之和，并导航到 `/reader/category/:categoryId`；文章深链接为 `/reader/category/:categoryId/entry/:entryId`。
- 继续使用 ASTRYX `TreeList`。分类管理使用一个 `Dialog purpose="form"`、`TextInput`、`Selector`、`List/Item`、`Button` 和删除 `AlertDialog`；不创建自定义 dropdown、modal 或 tree control。
- Dialog 只处理一个任务：创建/重命名/删除分类，以及把当前选中 Feed 移动到分类。不会在 v1 加拖拽、嵌套树、批量规则或颜色图标。
- 桌面、900px 和 compact 路由语义一致；移动端关闭 Dialog 后焦点回到打开按钮。分类选择、J/K/N/P/M/S 与路由切换不增加新动画。
- 删除当前分类后导航到 `/reader/unread`；移走当前 Feed 不强制关闭已打开文章，但下一次来源加载使用新分类投影。

## 7. 安全威胁模型

| 边界 | 滥用 | 控制 |
| --- | --- | --- |
| categoryId 输入 | IDOR、枚举其他用户分类 | 所有 repository 方法显式接收 userId；跨用户返回 404/空页；集成测试 |
| 分类标题 | 超长输入、控制字符、数据库排序差异 | Rust 边界校验；确定性 normalization；参数化查询；唯一约束 |
| subscription patch | 把自己的订阅绑定到他人分类 | 同一事务校验 category.user_id；更新语句同时限定 subscription.user_id |
| 删除分类 | 意外删除订阅或文章 | FK 只把 category_id 设 null；AlertDialog 明示影响；契约测试保留订阅/条目 |
| 并发创建 | 超过配额、重复标题 | user row lock、事务内 count/exact check、唯一约束 |
| cursor | 跨用户/跨分类重放 | 签名 frame 绑定 user + filter hash + snapshot |

## 8. 代码风格与测试策略

Rust 公共 DTO 使用 `category_id` 等 snake_case，HTTP 通过 serde camelCase。错误类型不携带原始标题、SQL 或跨用户 ID 到 Display/Debug。SQL 使用 SeaORM Statement 参数，不拼接用户值。

测试层次：

1. 三数据库 migration contract：列、FK、unique、index、cascade/set-null、down/up re-entry。
2. repository contract：校验、250 配额、重复、排序、并发创建、跨用户不可见、删除保留订阅。
3. router/OpenAPI：认证、CSRF、缓存头、method/fallback、状态码与真实响应 schema。
4. Reader query：category filter、cursor binding、feed/category 互斥、跨用户空页。
5. 前端 unit：生成 DTO、reducer、route parser、分组、Dialog mutations。
6. Playwright：1280×800、900×800、390×844、360×800 的分类创建、分配、深链接、Back、删除 fallback 与水平 containment。

## 9. 边界

始终执行：用户作用域查询、CSRF、参数化 SQL、OpenAPI drift gate、ASTRYX 优先、文件按功能拆分、提交前完整验证。

本切片不执行：嵌套分类、拖拽排序、标签、多选批量移动、来源搜索、用户设置、管理员用户管理、OIDC、OPML、AI/plugin/MCP。

绝不执行：通过分类删除 Feed/entry，向客户端暴露其他用户资源差异，手写第二份 TypeScript wire DTO，修改 `.superpowers/research/` 或根目录 `node_modules/`。

## 10. 完成标准

- SQLite/PostgreSQL/MySQL 共享 migration/repository 合同，分类删除后订阅仍存在且 `category_id = null`。
- 分类 CRUD、订阅 patch 和 category entry filter 的真实 router 与 committed OpenAPI 一致。
- 跨用户 category/subscription ID 无法读取或修改，错误不泄露存在性。
- Reader TreeList 展示分类与未分类 Feed，分类深链接/文章深链接/Back 在四视口工作。
- 前端 wire 类型全部由 committed OpenAPI 生成；Reader 仍只有一个 controller/store。
- 完整 Rust、TypeScript、Vitest、build、Playwright 和 `agent-browser` 验证通过后提交并推送。

## 11. 内部自审

- DDIA：记录系统、派生视图、并发仲裁、事务边界、索引和演进策略均已明确。
- API：资源名、输入输出、错误、分页例外和 additive 字段均已绑定。
- 安全：IDOR、跨租户绑定、并发配额、cursor 重放和删除破坏性均有对应测试。
- UX：分类只扩展现有 Reader 心智模型，不引入第二套状态或空壳功能。
- 范围：用户设置与管理员管理拆到后续独立切片，避免本计划跨越多个可独立评审子系统。
