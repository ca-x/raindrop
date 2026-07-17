# Raindrop Subscription API v1 Design

Date: 2026-07-17

## 1. Objective

交付从空用户状态开始的真实 RSS 闭环：用户提交 HTTPS Feed URL，系统原子创建或复用共享 Feed 与用户 Subscription，在后台安全抓取并持续刷新，客户端可观察订阅、未读数与稳定刷新状态，并直接使用已有 Reader entry API 阅读。

本切片吸收 CommaFeed 的共享 Feed/用户 Subscription、重复订阅复用、退订只移除用户关系和刷新状态可观察思想，并改进其同步双重抓取、GET mutation、进程内锁、异常文本泄漏和跨用户引用问题。

用户已把后续设计确认委托给内部评审；本规格完成一次多轴 review 和一次 bounded re-review 后直接实施。

## 2. Decisions and scope

- 先做后端 Subscription API 与后台 Feed runtime，再接 ASTRYX Reader。
- HTTP mutation 只提交幂等数据库命令，不等待 DNS、网络、解析、清洗或持久化。
- 首版是平面 Subscription 集合，路径统一使用 `/api/v1/subscriptions`；不把用户关系伪装成 `/feeds`。
- 本切片包含 list、detail/poll、create、manual refresh、unsubscribe 和后台 scheduled refresh。
- 暂不加入分类、重命名、拖放排序、OPML、批量已读、SSE/WebSocket 或物理 Feed 清理。
- 新 Subscription 可见最近 100 条已持久化条目且初始未读；窗口基于订阅事务锁定的 Feed head。
- 退订只删除用户 Subscription；最后一个订阅消失时标记 Feed orphan，不内联删除 Feed，running refresh 可安全完成但不能恢复关系。
- 不新增数据库迁移或 crate。现有 run idempotency、lease/fencing、Feed `orphaned_at` 和 timestamps 足够。
- live IT Home smoke 是人工、非阻塞发布证据；本地 deterministic fixture 与三数据库 contract 才是阻塞门。

## 3. Tech stack and commands

- Rust edition `2024`，`rust-version = 1.94`。
- Axum `0.8.9`、Tokio `1.52`、SeaORM `1.1.19`、SQLite/PostgreSQL/MySQL。
- 复用 `HttpFeedTransport`、`FeedUrlPolicy::new(false)`、parser/sanitizer、refresh claim/fencing 和 lifecycle outbox。

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features --test subscription_api
cargo test --locked --all-features --test feed_ingestion_e2e
cargo test --locked --all-features --test feed_refresh_claims
cargo test --locked --all-features --test openapi_contract
cargo test --locked --all-features --test reader_api
cargo test --locked --all-features
git diff --check

# 人工、非阻塞发布 smoke；记录结果，不作为 PR gate
RAINDROP_LIVE_RSS_SMOKE=1 cargo test --locked --all-features --test live_rss_ithome -- --ignored --nocapture
```

## 4. Project structure

```text
src/api/subscriptions.rs       scoped HTTP routes、wire DTO、错误/cache mapping
src/api/error.rs               additive retryAt/header-capable 429 与 conflict errors
src/api/mod.rs                 注册 subscriptions API
src/api/routes.rs              合并 scoped router
src/api/rate_limit.rs          bounded user-keyed mutation limiter
src/app.rs                     FeedRuntimeHandle、mutation limiter
src/main.rs                    启动/关闭 Feed runtime
src/feeds/dto.rs               page/outcome/refresh domain DTO
src/feeds/subscription.rs      list/detail/create/delete/manual command transactions
src/feeds/service.rs           FeedCommandService 与 FeedExecutor
src/feeds/runtime.rs           schedule、recovery、claim、heartbeat、shutdown
src/feeds/repository.rs        Feed-first queue/recovery primitives
src/feeds/mod.rs               导出稳定接口
docs/openapi/subscription-v1.json  committed OpenAPI fragment
tests/subscription_api.rs      router-level API/security/cache contracts
tests/feed_ingestion_e2e.rs    domain/runtime contracts
tests/feed_refresh_claims.rs   queue/recovery/lease contracts
tests/openapi_contract.rs      wire DTO/route/error 与 OpenAPI drift gate
```

HTTP handler 不执行 SQL、DNS 或抓取；runtime 不拥有 HTTP DTO；domain DTO 不添加 HTTP Serde derive。

## 5. Architecture

```text
Browser
  ├─ GET /subscriptions ─────────→ user-scoped query ─→ database
  ├─ GET /subscriptions/:id ─────→ user-scoped detail ─→ database
  ├─ POST /subscriptions ────────→ short command tx ───→ Feed + Subscription + optional run
  ├─ POST /subscriptions/:id/refresh → short command tx → queued run or typed rejection
  └─ DELETE /subscriptions/:id ─→ short command tx ───→ delete relation + maybe orphan Feed
                                                         │
                                                         └─ best-effort notify
                                                               ↓
FeedRuntime → stale-run recovery → due enqueue → claim_due + heartbeat/fencing
            → HttpFeedTransport → parse/sanitize → atomic persist + lifecycle outbox
```

数据库是记录系统。Notify 和 worker lanes 只降低延迟；Notify 关闭或丢失不能改变已提交命令的 HTTP 成功。进程重启后 runtime 从数据库恢复。

## 6. Stable domain boundaries

```text
FeedCommandService
  subscribe(user, input) -> SubscribeOutcome
  queue_subscription_refresh(user, subscription, request) -> RefreshDto
  unsubscribe(user, subscription) -> bool

FeedExecutor<T: FeedTransport>
  execute_claim(claim: RefreshClaim) -> Result<RefreshDto, FeedServiceError>

FeedRuntime
  only component allowed to recover, enqueue scheduled work, claim, heartbeat,
  invoke FeedExecutor, cancel attempts, and coordinate shutdown
```

`FeedExecutor::execute_claim` 只执行已经取得的 claim，不再次调用 `claim_run/claim_due`。现有 fetch/parse/persist 逻辑从 `FeedService::execute_run` 移入该唯一实现，禁止复制成第二条执行链。

## 7. Subscription queries

### 7.1 DTO

```rust
pub struct ListSubscriptionsQuery {
    pub cursor: Option<String>,
    pub limit: u16,
}

pub struct SubscriptionListItemDto {
    pub subscription_id: String,
    pub feed_id: String,
    pub title: String,
    pub site_url: Option<String>,
    pub unread_count: i64,
    pub refresh: Option<RefreshDto>,
}

pub struct SubscriptionPage {
    pub items: Vec<SubscriptionListItemDto>,
    pub next_cursor: Option<String>,
}
```

`list_subscriptions_for_user`：

- canonical user UUID；limit 默认 50、范围 `1..=100`；cursor 最大 1,024 bytes。
- 排序固定为 `(created_at DESC, subscription_id DESC)`；当前 `position` 全为 0，不进入 v1 wire/order。
- cursor v1 是 canonical URL-safe base64 frame，包含 version、user-bound hash、sort contract、last created-at micros 和 subscription ID；跨用户/排序版本/noncanonical reuse 返回 validation error。
- title precedence：non-empty title override → Feed title → normalized URL host。
- unread count 与 Reader 完全同构：只统计 `entry.feed_sequence > start_sequence` 且 `COALESCE(read_override, entry.feed_sequence <= read_through_sequence) = FALSE`。
- latest refresh 按 `(queued_at DESC, run_id DESC)`；没有 run 返回 `None`。

`get_subscription_for_user(user_id, subscription_id)` 使用同一映射；missing 与 cross-tenant 均返回 `None`。

## 8. Queue/create command

### 8.1 Initial history and quota

常量：

```text
INITIAL_VISIBLE_ENTRY_COUNT = 100
MAX_SUBSCRIPTIONS_PER_USER = 1000
MAX_ACTIVE_USER_REFRESH_RUNS = 20
```

所有用户 mutation 先在同一事务中锁 active user row，再锁 Feed row。用户锁内检查 Subscription 总数和 user-requested `QUEUED/RUNNING` runs；超过配额返回 typed capacity error。runtime/scheduled work不锁 user，只锁 Feed，因此锁顺序没有反向边。

新 Subscription 在 Feed lock 内读取 `entry_sequence_head`：

```text
initial_frontier = max(entry_sequence_head - 100, 0)
start_sequence = initial_frontier
read_through_sequence = initial_frontier
```

因此 head=60 时立即有 60 条未读，head=150 时只暴露最近 100 条。

### 8.2 Atomic Feed-first queue primitive

所有 SUBSCRIBE/MANUAL/SCHEDULED/IMPORT/RETRY enqueue 使用同一短事务原语：

1. 用户命令已持 user lock；所有路径随后锁 Feed row。PostgreSQL/MySQL `FOR UPDATE`；SQLite 第一条 Feed 业务语句为 scoped no-op `UPDATE` 获取 writer lock。
2. 锁后重新读取 disabled/orphan/due/lease 和 active run。
3. 在锁内查 exact idempotency key。
4. 根据调用模式决定返回 exact、返回/拒绝 active、插入新 run 或跳过。
5. scheduled 还必须验证扫描时的 `next_fetch_at` 与锁后值一致且仍 due。

唯一约束继续是最后仲裁；“先查后插”不在锁外发生。

### 8.3 Subscribe

```rust
pub struct SubscribeOutcome {
    pub created: bool,
    pub subscription: SubscriptionListItemDto,
}
```

流程：

- strict HTTPS URL，最大 4,096 bytes；完整 normalized URL 比较防 hash collision。
- 原子创建/复用 Feed 与 `(user, feed)` Subscription，清除 orphan marker。
- 重复 POST 已有 Subscription 只返回现有资源，不改标题/排序，不创建 SUBSCRIBE run。
- 新 Subscription 仅当 Feed 新建、从未成功抓取或锁后 `next_fetch_at <= database now` 时需要 refresh。
- 需要 refresh 时：若已有 `QUEUED/RUNNING` run，Subscription response 引用该状态；否则插入稳定 `subscribe:{subscription_id}` run。该 key 长度不超过 64。
- 提交后 best-effort notify；不等待网络。

`created` 只表示本次是否创建用户 Subscription。

## 9. Manual refresh command

请求包含 canonical UUID `request_id`。idempotency key 使用版本化、带边界帧的 BLAKE3 摘要：

```text
key = "m1:" + BASE64URL_NO_PAD(BLAKE3(frame(user_id, request_id)))
```

总长度 46 bytes，不超过现有 `VARCHAR(64)`；raw user/request ID 不出现在 key/log/error。

锁内判定顺序必须精确：

1. 查 exact key；存在则验证 requested user/trigger 语义并返回同一 run，即使它已 terminal。该检查优先于 cooldown。
2. 若存在其它 `QUEUED/RUNNING` run，本请求不被接受，返回 `RefreshInProgress { operation_id }`；HTTP 409。因为命令未提交，不承诺该 requestId 的幂等映射。
3. cooldown：`retry_at = max(last_attempt_at + 30s, feed.retry_after_at)`；未到期返回 typed cooldown。
4. 检查 user active-run capacity，随后插入 MANUAL run。

同一 accepted request 永远返回同一 run；不同 request 不会在同一 Feed 上并发排队。

## 10. Unsubscribe

`unsubscribe(user_id, subscription_id) -> bool` 是幂等、不可枚举命令：

1. canonical UUID；事务外只读获得候选 feed ID，缺失直接返回 false。
2. 事务内锁 active user，再锁候选 Feed，重新按 `(subscription_id,user_id,feed_id)` 检查。
3. 删除用户 Subscription；不删除 entries/states/feed/runs。
4. 若 Feed 已无任何 Subscription，设置 `orphaned_at=database now`；否则保持 null。
5. 并发重新订阅同一 Feed 通过相同 user→Feed 锁顺序串行；subscribe 最终清除 orphan marker。

running refresh 可完成；scheduled enqueue 排除 orphan/no-subscription Feed。worker 绝不插入 Subscription，因此完成不能恢复关系。

## 11. Refresh domain and public projection

`RefreshDto` additive 加入内部运行信息：error/retry/queued/started/completed timestamps。HTTP 不直接冻结内部 `RefreshStatus`，而映射稳定 `refresh.state`：

| internal | public state |
| --- | --- |
| QUEUED / RUNNING | `PENDING` |
| SUCCESS / NOT_MODIFIED | `READY` |
| PARTIAL | `DEGRADED` |
| ERROR with retry_at | `BACKING_OFF` |
| ERROR without retry / LEASE_LOST / CANCELLED | `ERROR` |

公开 refresh：

```text
operationId, state, newCount, updatedCount, droppedCount, generation,
errorCode?, retryAt?, queuedAt, startedAt?, completedAt?
```

公开 errorCode 只允许 `REFRESH_FAILED | UPSTREAM_RATE_LIMITED`；其它持久化内部 error code 归一化为 `REFRESH_FAILED`。时间统一为 UTC RFC3339、固定六位微秒和 `Z`。

## 12. Feed runtime and crash recovery

### 12.1 Runtime lifecycle

`FeedRuntimeHandle` 暴露 best-effort `notify()` 和 shutdown token。runtime 持有 `SetupService`；setup 未完成时零网络，每秒/Notify 重查 database。ready 后创建 repository、`FeedExecutor<HttpFeedTransport>` 并启动 2 个 worker lanes。

lane 顺序：recover stale → enqueue due (scheduler lane only) → claim due → execute with heartbeat。没有工作时等待 Notify 或最多 1 秒。

### 12.2 Heartbeat coordination

- lease 60 秒；每 20 秒 extend。
- attempt 与 heartbeat 用结构化并发协调。attempt 先 terminal 成功时先停止并 join heartbeat，再返回；迟到 heartbeat 不得把成功误判为 lease loss。
- heartbeat 返回 LeaseLost 时取消 attempt，并调用数据库 recovery primitive；其它 heartbeat repository error 同样取消 attempt并记录脱敏日志，但不伪造 terminal success。
- shutdown 停止新 claim，等待当前 attempt 最多 30 秒；超时取消，交给 stale recovery。

### 12.3 Stale RUNNING recovery

当前 `claim_due` 只接受 QUEUED，因此 runtime 必须主动恢复过期 RUNNING：

1. 扫描 lease expired/token mismatch 的 RUNNING candidates。
2. 每个 candidate 在 Feed-first 锁事务中重新验证 run status、feed lease/token。
3. 把旧 run 标记 `LEASE_LOST` 和 completed_at。
4. 若同 Feed 已有另一个 QUEUED/RUNNING run，不再插入。
5. 否则幂等插入 trigger `RETRY`，key=`r1:{old_run_id}`（39 bytes），requested user 继承旧 run。
6. 事务提交后 Notify。

worker crash、进程 kill 或 heartbeat 丢失最终都通过数据库扫描产生 RETRY；不声称原 run 会被重新 claim。

### 12.4 Scheduled enqueue

每 30 秒扫描最多 100 个候选：due、not disabled、not orphan、至少一个 Subscription。对每个候选进入 atomic Feed-first queue primitive，锁后重新验证 `next_fetch_at` 与扫描版本且仍 due，无 active run 才插入：

```text
key = "s1:" + BASE64URL_NO_PAD(BLAKE3(frame(feed_id, next_fetch_at_micros)))
```

多实例扫描依靠 Feed lock 和 exact key 收敛。内存 tick 不是调度权威。

## 13. HTTP API

### 13.1 Endpoints

```text
GET    /api/v1/subscriptions?cursor=&limit=
GET    /api/v1/subscriptions/{subscription_id}
POST   /api/v1/subscriptions
POST   /api/v1/subscriptions/{subscription_id}/refresh
DELETE /api/v1/subscriptions/{subscription_id}
```

GET 使用 `CurrentUser`。Mutation extractor：

```text
create:  CurrentUser → CsrfGuard → ApiJson
refresh: CurrentUser → CsrfGuard → Path → ApiJson
delete:  CurrentUser → CsrfGuard → Path
```

`UserMutationLimiter::check(user_id)` 在所有 extractor/业务 validation 成功后、command service 前执行，因此 401/403/422 优先于 429。两个 mutation POST 与 DELETE 共用每用户 30 次/15 分钟的有界 keyed limiter；一个用户不能耗尽另一个用户的额度，最多保留 10,000 个活跃 key，过期清理。

### 13.2 List/detail wire

```json
{
  "subscriptionId": "...",
  "feedId": "...",
  "title": "IT之家",
  "siteUrl": "https://www.ithome.com/",
  "unreadCount": 60,
  "refresh": {
    "operationId": "...",
    "state": "READY",
    "newCount": 60,
    "updatedCount": 0,
    "droppedCount": 0,
    "generation": 1,
    "errorCode": null,
    "retryAt": null,
    "queuedAt": "2026-07-17T12:00:00.000000Z",
    "startedAt": "2026-07-17T12:00:00.100000Z",
    "completedAt": "2026-07-17T12:00:01.000000Z"
  }
}
```

list response 为 `{ items, nextCursor }`。detail missing/cross-tenant 同一 404。

### 13.3 Create

请求 `{ "url": "https://www.ithome.com/rss/" }`，strict camelCase/deny unknown。新 Subscription 返回 201、`Location: /api/v1/subscriptions/{id}`；已有返回 200。响应 `{ created, subscription }`。合法初始 refresh 可以已经是 PENDING/READY/BACKING_OFF；不使用墙钟“必须 QUEUED”断言。

### 13.4 Refresh

请求 `{ "requestId": "canonical UUID" }`。accepted queued/running 返回 202；exact idempotent replay 已 terminal 返回 200。其它 active run 返回 409 `REFRESH_IN_PROGRESS`，`error.fields.operationId`；idempotency semantic conflict 返回 409 `CONFLICT`。

temporal limiter/cooldown 返回统一 429，并携带可证明的 retry metadata：

```json
{
  "error": {
    "code": "RATE_LIMITED",
    "message": "Too many requests",
    "fields": { "retryAt": "2026-07-17T12:00:30.000000Z" },
    "requestId": "..."
  }
}
```

`Retry-After` 是相对 repository transaction 已读取的 database now 或 limiter now 向上取整的整数秒，最小 1。HTTP 只格式化 repository 提供的 persisted `retry_at` 和 `retry_after_seconds`，不得再用 app wall clock 重算。exact idempotency lookup 优先于 cooldown/limiter database admission；HTTP memory limiter 仍在 command 前执行，因此 client 应在正常限额内重试。

`SubscriptionLimit` / `ActiveRefreshLimit` 是没有可证明解除时刻的 hard quota，仍返回 429 `RATE_LIMITED` 和稳定 message `Too many requests`，但省略 `fields.retryAt` 与 `Retry-After`；不得伪造 app-now-based retry time。

### 13.5 Delete

well-formed ID 无论 missing、cross-tenant 或已删除都返回 204，避免枚举并保持幂等。invalid UUID 422；认证/CSRF 仍按 401/403。

### 13.6 Errors/cache/router

| condition | HTTP | code |
| --- | ---: | --- |
| invalid query/path/body/URL/UUID/cursor/limit | 422 | `VALIDATION_ERROR` |
| unauthenticated/expired/disabled | 401 | `AUTHENTICATION_REQUIRED` |
| CSRF/Origin/Host | 403 | `FORBIDDEN` |
| GET detail missing/cross-tenant | 404 | `NOT_FOUND` |
| refresh active/idempotency/hash/disabled conflict | 409 | typed conflict |
| limiter/quota/cooldown/backlog | 429 | `RATE_LIMITED` |
| DB/corrupt internal | 500 | `INTERNAL_ERROR` |

Durable command commit 不依赖 worker liveness，因此无 worker/claim 503。

使用一个 `/api/v1/subscriptions` scoped nested router：inner `/`、`/{id}`、`/{id}/refresh`，inner fallback/method fallback；outer 显式处理 `/api/v1/subscriptions/` trailing slash；cache middleware 包围整个 scoped router。所有 success/error/404/405 带 JSON（204 除外）和 `Cache-Control: no-store`、`Pragma: no-cache`，不落入 embedded Web。

## 14. OpenAPI contract

本切片不加 generator crate。提交 `docs/openapi/subscription-v1.json`，包含五个 endpoint、request/response/error schema、enum、status 和 header。`tests/openapi_contract.rs`：

- 用真实 router fixture 对每个 documented success/error 生成响应并按 schema fixture 的 required/type/enum 校验。
- 断言所有实际 route/status/header 在 artifact 中存在，artifact 没有未实现 endpoint。
- artifact 作为下一 UI 切片的唯一 DTO 输入；TypeScript generation/drift gate在首次 Reader API client task中读取该 artifact，不能手写第二份 wire contract。

## 15. Security and DDIA review

- SSRF：handler 永不构造 reqwest；runtime 固定 HTTPS policy、DNS pin、redirect/peer revalidation 和现有 budgets。
- Abuse：64 KiB body、4,096-byte URL、limit 100、1000 subscriptions/user、20 active user runs、30 mutations/15m、2 worker lanes、单 Feed lease。
- Authorization：user ID 只来自 CurrentUser；共享 Feed/run ID 不是授权；GET detail统一 404，DELETE统一204。
- Record system：database owns commands/runs/lease/outbox；Notify/lanes/ticks 可丢失。
- Idempotency：URL hash+full URL、`(user,feed)` unique、versioned digest keys、exact semantic validation。
- Concurrency：所有 enqueue/recovery 为 Feed-first locked transaction；user commands为 user→Feed，runtime不锁 user；无反向锁序。
- Recovery：RUNNING 不被重新 claim；expired run 原子 LEASE_LOST→optional RETRY。
- Read-after-write：POST 后 GET 立即看到 Subscription/PENDING；不承诺 entries 已完成。
- Snapshot：初始 100 条窗口来自锁定 head；Reader cursor继续绑定 ingestion generation。
- At-least-once：retry可重复调度，但 identity/fencing/outbox idempotency 收敛；不声明 end-to-end exactly-once。
- Feed/plugin：网络/parse/AI/MCP/plugin 不进入数据库事务；AI/plugin只消费 post-commit outbox。

## 16. Testing strategy

Repository/domain：

- list/detail/cursor user binding/title/unread mapping。
- head 60→60 unread；head 150→100 unread。
- subscription quota与 user active-run cap 在 user lock下并发安全。
- same-user/two-user concurrent subscribe；fresh shared Feed不重复抓取；due shared Feed只插一个 active run。
- manual exact replay、different request active 409、digest key长度、cooldown=max(30s,retry_after)。
- unsubscribe missing/cross-tenant 204、last relation orphan、concurrent resubscribe clears orphan。

Runtime：

- setup required零网络；ready后Notify与poll都能驱动。
- two lanes不同 Feed并发、同 Feed不并发。
- heartbeat extend；terminal先停heartbeat；LeaseLost取消attempt。
- crash fixture留下expired RUNNING，recovery标LEASE_LOST并唯一创建RETRY，最终完成。
- scheduled scan revalidation、multi-instance race、disabled/orphan/no-subscription/active skip。

HTTP：

- list/detail/create/refresh/delete 200/201/202/204/409/429与Location/Retry-After/cache。
- 401/403/422 precedence；limiter user isolation；missing/cross-tenant contracts。
- unknown/trailing slash JSON404；known wrong method JSON405；no embedded HTML。
- blocking fake transport证明 POST 在网络 gate释放前返回；不要用墙钟快速断言。

Live smoke（非阻塞）：

- `https://www.ithome.com/rss/` 最终 50..=100 entries；当前预期观察约60。
- 第二次允许 `NOT_MODIFIED/304`，或 `SUCCESS/200` 且 identity 去重后无重复条目。
- 不提交 Feed 内容、URL query、header或响应正文。

## 17. Boundaries

Always：Rust 2024、MSRV 1.94、Cargo `--locked`；三数据库一致；用户作用域；fresh fmt/clippy/focused/full/diff；本地无 server URL时skip但CI真实运行。

Internally review before changing：migration/dependency、100 history、1000 quota、20 active runs、2 lanes、60s lease/20s heartbeat、30s cooldown、endpoint/status/cache/auth/CSRF。

Never：修改用户未跟踪 `.superpowers/research/`/`node_modules/`；HTTP等待网络；GET mutation；raw exception/URL/body/SQL泄漏；ID替代授权；内存lock/queue作为唯一正确性机制。

## 18. Success criteria

- 空用户通过 API 创建、轮询、阅读、刷新和退订 Subscription。
- POST 不等待网络，client disconnect不丢命令。
- duplicate subscribe/manual retry/multi-user/multi-instance queue按合同收敛。
- 新用户订阅已有 Feed立即获得最多100条稳定未读历史。
- stale RUNNING最终通过唯一RETRY恢复；scheduled refresh持续工作。
- auth/CSRF/tenant/SSRF/quota/rate/cache/error/OpenAPI drift contracts通过。
- synthetic、三数据库、full suite和bounded review为阻塞门；live IT Home smoke记录为非阻塞发布证据。

## 19. Open questions

无阻塞问题。分类/排序编辑、物理 Feed retention、OPML、批量已读、SSE/WebSocket和ASTRYX Reader UI拆到后续切片。
