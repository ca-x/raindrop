# Raindrop Content Jobs / Artifacts Core v1 设计

日期：2026-07-19

状态：内部评审通过，实施绑定

关联规格：

- `docs/superpowers/specs/2026-07-16-raindrop-design.md`
- `docs/superpowers/specs/2026-07-17-ai-plugin-v1-design.md`
- `docs/ai-providers.md`

## 1. 决策摘要

本切片建立 AI 内容能力的持久化执行内核：`content_jobs` 是待执行意图及其状态机的记录系统，`content_job_attempts` 是每次有界执行的不可覆盖审计记录，`content_artifacts` 是不可变派生内容，`content_job_results` 把每个成功 job 显式连接到新建或复用的 artifact。

数据库负责幂等入队、可恢复 lease、单调 fencing token、attempt 编号、终态、artifact identity 去重和原子提交。worker、Wasm 内存、provider response、MCP transport 和 HTTP 请求都不是恢复依据。投递和执行采用至少一次语义，通过 idempotency key、claim fencing 与 artifact identity 把重复执行收敛为同一持久化结果。

本切片只实现通用 storage/domain contract，不执行 Wasm、provider 或 MCP，不新增 HTTP route、Reader UI、自动 lifecycle dispatcher 或 provider 管理 UI。后续所有摘要/翻译入口必须复用该 core；禁止 native processor 或 route handler 直接调用模型。

## 2. 假设、目标与排除范围

### 2.1 假设

- Rust edition 保持 2024，支持 SQLite、PostgreSQL、MySQL 三个 backend。
- `users`、`subscriptions`、`entries` 和 `ai_providers` 已存在；entry 的 `content_hash` 是 sanitized content snapshot 的稳定版本标识。
- job ID、attempt ID、artifact ID 使用 UUID v4 字符串；所有 hash 使用带 domain separation 的 framed BLAKE3 lower-hex，共 64 个 ASCII 字符。
- 数据库时间是 lease、deadline 和 retry due 判断的权威时间；进程 wall clock 只用于本地 timeout，不决定数据库所有权。
- 单个 operation 只产生一个主 artifact。未来需要多个输出时通过新 operation/schema 或 additive result role 扩展，不在 v1 暗含多结果语义。

### 2.2 本切片目标

- 用户作用域的幂等 job enqueue；同 key 同请求返回原 job，同 key 不同请求稳定冲突。
- enqueue 时验证 active user、entry 存在、用户可见性和 entry content hash snapshot。
- 可恢复 claim、lease heartbeat、attempt 审计、fencing 与每用户最多两个 RUNNING jobs。
- retryable failure 使用最多三次 attempt 的有界指数退避；永久失败直接终止。
- 成功 artifact 按完整 identity 复用，artifact 永不覆盖 entry 原文。
- artifact insert / result link / attempt success / job success 在同一短事务提交。
- SQLite、PostgreSQL、MySQL 运行同一 repository contract suite。
- 所有错误、metadata 和 provenance 都使用安全、bounded、无 secret 的持久化合同。

### 2.3 明确排除

- plugin installation/config/capability grant、WIT、Wasmtime 与官方 Wasm component。
- provider 配置 API/UI、credential contract probe、模型调用和 quota reservation。
- MCP connection、tool broker、tool-call audit 与 Raindrop MCP server。
- lifecycle delivery fan-out、自动规则、Reader sidecar、OpenAPI 和前端展示。
- 用户取消、优先级队列、批量 job、跨 operation DAG、跨数据库事务或外部 broker。
- artifact 删除、retention、重新生成 UI 和旧 artifact 的迁移/重写。

## 3. DDIA 内部评审结论

### 3.1 记录系统与派生数据

`entries.sanitized_content` 和 `entries.content_hash` 属于 RSS core 记录系统。AI artifact 是可重建派生数据，失败、延迟或 stale 都不能影响原文、已读、收藏和 Feed 刷新事务。

`content_jobs`、`content_job_attempts`、`content_job_results` 与 artifact provenance 是执行事实的记录系统。内存 channel 只能作为 wake-up hint；进程崩溃后 worker 必须只依赖数据库重建工作集合。

### 3.2 至少一次而不是虚假的 exactly-once

网络 timeout 无法证明 provider 是否完成，进程也可能在 provider 返回后、数据库 commit 前崩溃。因此系统不声称 provider 调用 exactly-once：

1. job claim 可能因 lease expiry 被再次执行；
2. provider 调用可能出现 unknown outcome；
3. artifact identity 唯一约束与原子终态事务保证最终只有一个有效派生结果；
4. attempt audit 保留重复调用、成本和 unknown outcome，而不伪装为从未发生。

### 3.3 安全性优先

- lease token 每次 claim 单调增加，旧 worker 即使恢复也不能提交。
- completion 同时验证 job ID、user ID、attempt、lease owner、lease token、`lease_until > database_now` 与 `attempt_deadline_at > database_now`。
- artifact 是 immutable；current/stale 是相对于当前 identity 的读时判断，不使用可漂移的 `is_current` 布尔列。
- 所有跨表终态写入使用一个本地数据库事务，不使用 2PC。

### 3.4 可演进性

状态、trigger、operation、kind 和 schema ID 在 Rust 侧是封闭 v1 enum，但数据库使用 bounded string，未来只 additive 增加值。JSON 字段必须带 schema/version 信息或由外层固定版本解释；旧 reader 忽略未知可选字段。破坏性 schema 变更遵循 expand → backfill → switch → later contract。

## 4. 模块边界

新增 `src/content/jobs/`，按职责拆分：

```text
src/content/jobs/
├── mod.rs          public exports
├── model.rs        validated enums, requests, claims, artifacts, errors
├── hash.rs         framed/domain-separated canonical hashes
├── repository.rs   backend-neutral orchestration and transactions
└── sql.rs          backend-specific SQL and row decoding
```

约束：

- `model.rs` 不依赖 SeaORM entity active model。
- `hash.rs` 不访问数据库或 provider secret。
- `repository.rs` 不调用 provider、MCP、Wasm、HTTP 或 Feed parser。
- `sql.rs` 只包含 SQLite/PostgreSQL/MySQL 的锁、数据库时钟、条件更新与 row decode 差异。
- public API 不暴露 `DatabaseTransaction`、供应商 DTO、secret 或任意 JSON 日志正文。

## 5. 状态机

### 5.1 Job 状态

```text
QUEUED ────────────── claim ─────────────► RUNNING
   ▲                                           │
   │                    retryable failure      │
   └──────── RETRY_WAIT ◄──────────────────────┤
                                               │
                                               ├── success/reuse ─► SUCCEEDED
                                               ├── permanent error ─► FAILED
                                               └── attempts exhausted ─► FAILED

RUNNING -- expired lease/deadline --> recovery --> RUNNING(next attempt)
RUNNING -- expired at max attempts -----------> FAILED
```

- 非终态：`QUEUED`、`RUNNING`、`RETRY_WAIT`。
- 终态：`SUCCEEDED`、`FAILED`。
- v1 没有 `CANCELLED`；不得预先实现没有 API 语义的状态。
- enqueue 命中完整 artifact identity 时创建 `SUCCEEDED` job、零 attempt 和一条 reused result link。
- `attempts` 是已分配的 attempt 数，不是失败数；范围 `0..=max_attempts`。
- `started_at` 在首次 claim 设置后不改；`completed_at` 只在终态设置。

### 5.2 Attempt 状态

- `RUNNING`：当前 claim 创建，尚无终态。
- `SUCCEEDED`：与 job success 和 result link 同事务写入。
- `FAILED`：worker 持有有效 claim 时报告的 provider/plugin/MCP/validation failure。
- `ABANDONED`：下一个 claimer 发现前一 lease/deadline 已过期；`error_code = JOB_LEASE_EXPIRED`，`retryable = true`。

attempt 行只允许从 `RUNNING` 单向转为一个终态，不覆盖历史计量。唯一 `(job_id, attempt)` 防止同一编号重复。

## 6. 数据模型

所有时间列使用现有 `operational_timestamp`：SQLite/PostgreSQL 为带时区 timestamp 语义，MySQL 为 UTC `datetime(6)`。所有 JSON 物理存储为 `TEXT`，进入仓储前做 UTF-8、schema/canonical/size 验证。

### 6.1 `content_jobs`

| 字段 | 类型/空值 | 语义 |
| --- | --- | --- |
| `id` | string(36) PK | UUID v4 |
| `user_id` | string(36) NOT NULL FK users CASCADE | tenant owner |
| `entry_id` | string(36) NOT NULL FK entries CASCADE | source entry |
| `operation` | string(32) NOT NULL | `SUMMARIZE` / `TRANSLATE` |
| `artifact_kind` | string(32) NOT NULL | `AI_SUMMARY` / `AI_TRANSLATION` |
| `target_locale` | string(35) NULL | canonical BCP 47；summary 可空 |
| `trigger_kind` | string(32) NOT NULL | MANUAL_API / READER_SIDECAR / FEED_REFRESH_PERSISTED / MCP_SERVER |
| `plugin_key` | string(128) NOT NULL | v1 固定 `raindrop.ai-content`，合同保持通用 |
| `plugin_version` | string(64) NOT NULL | semver snapshot |
| `component_digest` | string(64) NOT NULL | lower-hex SHA-256 release digest |
| `provider_binding_id` | string(36) NOT NULL | snapshot reference；不设 FK，保留 provider 删除后的历史 |
| `provider_kind` | string(40) NOT NULL | adapter kind snapshot |
| `provider_model` | string(200) NOT NULL | model snapshot |
| `provider_revision` | bigint NOT NULL | provider config revision snapshot |
| `prompt_version` | string(64) NOT NULL | prompt contract snapshot |
| `schema_id` | string(255) NOT NULL | expected artifact schema |
| `entry_content_hash` | string(64) NOT NULL | enqueue 时 entries.content_hash |
| `input_hash` | string(64) NOT NULL | canonical invocation input snapshot hash |
| `config_hash` | string(64) NOT NULL | canonical effective config hash |
| `mcp_provenance_hash` | string(64) NOT NULL | disabled 也使用固定 empty provenance hash |
| `artifact_identity_hash` | string(64) NOT NULL | 完整 artifact identity hash |
| `idempotency_key` | string(255) NOT NULL | 原始 ASCII opaque key，用于 collision check/audit |
| `idempotency_key_hash` | string(64) NOT NULL | case-sensitive physical unique key |
| `request_hash` | string(64) NOT NULL | 同 idempotency key 的语义冲突检测 |
| `call_chain_id` | string(64) NOT NULL | recursion/audit scope |
| `remaining_depth` | integer NOT NULL | v1 `0..=4` |
| `status` | string(16) NOT NULL | job state |
| `attempts` | integer NOT NULL default 0 | allocated attempts |
| `max_attempts` | integer NOT NULL default 3 | enqueue snapshot，v1 固定 3 |
| `timeout_seconds` | integer NOT NULL | manual 180 / automatic 120 |
| `next_attempt_at` | operational timestamp NOT NULL | due time；initial = created_at |
| `lease_owner` | string(64) NULL | current worker ID |
| `lease_token` | bigint NOT NULL default 0 | monotonic fencing token |
| `lease_until` | operational timestamp NULL | renewable short lease |
| `attempt_deadline_at` | operational timestamp NULL | 当前 attempt 硬截止时间，不可续期 |
| `last_error_code` | string(64) NULL | safe summary only |
| `created_at` | operational timestamp NOT NULL | enqueue time |
| `started_at` | operational timestamp NULL | first claim |
| `completed_at` | operational timestamp NULL | terminal time |

物理约束与索引：

- unique `(user_id, idempotency_key_hash)`；读取命中后必须 constant-time 比较原 key，并比较 `request_hash`。
- `idx_content_jobs_due(status, next_attempt_at, lease_until, created_at, id)`。
- `idx_content_jobs_user_status(user_id, status, created_at, id)`。
- `idx_content_jobs_entry(user_id, entry_id, artifact_kind, target_locale, created_at, id)`。
- `idx_content_jobs_identity(user_id, artifact_identity_hash, status)`。

使用 hash 而不是直接索引 idempotency key，是为了在 MySQL 默认 case-insensitive collation 下仍保持 HTTP opaque key 的大小写语义，并控制跨 backend 索引宽度。hash collision 时 raw key 不同必须返回 `HASH_COLLISION`，不能误复用。

### 6.2 `content_job_attempts`

| 字段 | 类型/空值 | 语义 |
| --- | --- | --- |
| `id` | string(36) PK | attempt UUID |
| `job_id` | string(36) NOT NULL FK jobs CASCADE | parent job |
| `attempt` | integer NOT NULL | one-based sequence |
| `lease_token` | bigint NOT NULL | claim fencing snapshot |
| `status` | string(16) NOT NULL | RUNNING/SUCCEEDED/FAILED/ABANDONED |
| `started_at` | operational timestamp NOT NULL | DB claim time |
| `deadline_at` | operational timestamp NOT NULL | 该 attempt 的不可续期硬截止时间 |
| `completed_at` | operational timestamp NULL | attempt terminal time |
| `error_code` | string(64) NULL | stable safe code |
| `retryable` | boolean NULL | terminal classification；RUNNING 为空 |
| `outcome_unknown` | boolean NOT NULL default false | transport 已发送但结果未知 |
| `provider_request_count` | integer NOT NULL default 0 | max 3 |
| `mcp_call_count` | integer NOT NULL default 0 | trigger/config ceiling 内 |
| `input_tokens` | bigint NOT NULL default 0 | nonnegative measured/estimated |
| `output_tokens` | bigint NOT NULL default 0 | nonnegative measured/estimated |
| `estimated_cost_micros` | bigint NOT NULL default 0 | nonnegative |
| `execution_metadata_json` | text NOT NULL | canonical bounded redacted object |

约束与索引：

- unique `(job_id, attempt)`。
- `idx_content_attempts_job(job_id, started_at, id)`。
- execution metadata 最大 32 KiB；禁止 prompt、正文、provider body、credential、raw MCP args/result 和 stack。

### 6.3 `content_artifacts`

| 字段 | 类型/空值 | 语义 |
| --- | --- | --- |
| `id` | string(36) PK | artifact UUID |
| `user_id` | string(36) NOT NULL FK users CASCADE | tenant owner |
| `entry_id` | string(36) NOT NULL FK entries CASCADE | source entry |
| `producer_job_id` | string(36) NOT NULL FK jobs RESTRICT | 首次生成该 artifact 的 job |
| `kind` | string(32) NOT NULL | AI_SUMMARY / AI_TRANSLATION |
| `locale` | string(35) NULL | canonical target locale |
| `schema_id` | string(255) NOT NULL | payload schema |
| `entry_content_hash` | string(64) NOT NULL | source snapshot |
| `input_hash` | string(64) NOT NULL | canonical input hash |
| `config_hash` | string(64) NOT NULL | effective config hash |
| `processor_key` | string(128) NOT NULL | plugin key |
| `processor_version` | string(64) NOT NULL | plugin version |
| `component_digest` | string(64) NOT NULL | component snapshot |
| `provider_binding_id` | string(36) NOT NULL | historical binding label |
| `provider_kind` | string(40) NOT NULL | historical adapter kind |
| `provider_model` | string(200) NOT NULL | historical model |
| `provider_revision` | bigint NOT NULL | historical config revision |
| `provider_label` | string(200) NOT NULL | safe display label；不参与 identity |
| `prompt_version` | string(64) NOT NULL | prompt snapshot |
| `mcp_provenance_hash` | string(64) NOT NULL | bounded provenance digest |
| `identity_hash` | string(64) NOT NULL | canonical complete identity |
| `payload_json` | text NOT NULL | canonical schema-validated result |
| `provenance_json` | text NOT NULL | canonical bounded redacted provenance |
| `payload_size_bytes` | integer NOT NULL | UTF-8 byte length |
| `created_at` | operational timestamp NOT NULL | immutable creation time |

约束与索引：

- unique `(user_id, identity_hash)`；命中后比较所有 identity columns，collision 时 fail closed。
- `idx_content_artifacts_entry(user_id, entry_id, kind, locale, created_at, id)`。
- `idx_content_artifacts_producer(producer_job_id)`。
- `payload_json` 最大 512 KiB canonical UTF-8；`provenance_json` 最大 32 KiB。
- 没有 `updated_at`、`is_current`、`is_stale` 或正文覆盖列。

### 6.4 `content_job_results`

| 字段 | 类型/空值 | 语义 |
| --- | --- | --- |
| `job_id` | string(36) PK FK jobs CASCADE | 每个成功 job 恰好一个 result |
| `artifact_id` | string(36) NOT NULL FK artifacts RESTRICT | 新建或复用 artifact |
| `was_reused` | boolean NOT NULL | 是否由既有 identity 复用 |
| `linked_at` | operational timestamp NOT NULL | terminal transaction time |

索引 `idx_content_job_results_artifact(artifact_id, job_id)` 支持 provenance/read model。单独 result 表避免 jobs/artifacts 循环 FK，并让复用关系显式且可验证。

## 7. Canonical hashing

所有 hash 使用 length-prefixed frames，不能简单字符串拼接。每类 hash 使用独立 context：

```text
raindrop.content-job.idempotency.v1
raindrop.content-job.request.v1
raindrop.content-job.input.v1
raindrop.content-artifact.identity.v1
raindrop.content-mcp.provenance.v1
```

frame 格式是 `u64 big-endian length || bytes`，字段按规格固定顺序编码；nullable 值先编码 presence byte，再编码内容。枚举使用 storage value，locale 使用 canonical BCP 47，JSON 先 parse、递归按 key 排序并 compact serialize。

`request_hash` 精确覆盖 artifact identity 的全部字段，以及 `trigger_kind`、`call_chain_id`、`remaining_depth`、`max_attempts` 和 `timeout_seconds`；不覆盖 idempotency key、job ID 或时间。这样同一个 key 不能被重用于不同 trigger、递归预算或执行限制。

artifact identity 精确覆盖：

```text
user_id, entry_id, artifact_kind, target_locale,
entry_content_hash, input_hash, config_hash,
plugin_key, plugin_version, component_digest,
provider_binding_id, provider_kind, provider_model, provider_revision,
prompt_version, schema_id, mcp_provenance_hash
```

`provider_label` 不参与 identity，因为纯展示重命名不应单独改变内容语义；provider revision/model/kind 已固定执行语义。任一 identity 字段变化使旧 artifact 在新上下文中 stale，但旧行继续可读和审计。

## 8. Repository contract

### 8.1 Enqueue

`ContentRepository::enqueue(EnqueueContentJob) -> EnqueueResult`：

1. 完整验证 ID、enum、locale、版本、hash、idempotency key、call chain、timeout 和 limits。
2. 开启短事务，按 `User → Job/Artifact` 顺序获取锁。PostgreSQL/MySQL 对 user `SELECT ... FOR UPDATE`；SQLite 用 `UPDATE users SET id = id WHERE id = ?` 获取 writer serialization。
3. 验证 user active，并以 `subscriptions.user_id + entries.feed_id` 检查 entry 可见；不存在、无权和 disabled user 统一 `NotFound`。
4. 锁内读取 entry，要求 `entry.content_hash == entry_content_hash`；不一致返回 `EntryChanged`，调用方重建 snapshot/idempotency request。
5. 按 `(user_id, idempotency_key_hash)` 查询：raw key hash collision → `HashCollision`；request hash 相同 → 返回 existing；不同 → `IdempotencyConflict`。
6. 查询完整 artifact identity。命中且 identity columns 全等时，插入 `SUCCEEDED` job 与 `content_job_results(was_reused=true)`；否则插入 `QUEUED` job。
7. commit 后才返回。任何失败 rollback，不产生半个 job/result。

### 8.2 Claim 与恢复

`claim_next(ClaimContentJob) -> ClaimOutcome` 使用数据库 due time：

- candidate 是 `QUEUED/RETRY_WAIT` 且 `next_attempt_at <= now`，或 `RUNNING` 且 lease/deadline 已过期。
- read-only candidate query 先排除已经有两个有效 RUNNING jobs 的用户，再按 due/create/id 稳定排序并最多取 16 行；该过滤只改善 liveness，每个候选进入事务后仍先锁 user、再锁 job并重新验证，避免把无锁 count 当作并发正确性依据。
- 锁住 user 后统计有效 `RUNNING`（lease/deadline 均未过期）；达到 2 时跳过该用户。
- recovery 先把旧 RUNNING attempt 标成 ABANDONED。若已达 max attempts，同事务把 job 标成 FAILED/`JOB_ATTEMPTS_EXHAUSTED`，返回 `RecoveredTerminal`；否则分配下一 attempt。
- 新 claim：`lease_token += 1`、`attempts += 1`、status RUNNING、设置 owner、短 lease、不可续期的 attempt deadline，并插入 RUNNING attempt。
- token 达到 `i64::MAX` 时 job 以 `JOB_FENCE_EXHAUSTED` 失败，不能 wrap。

初始 lease 30 秒；heartbeat 每次最多续到 `min(database_now + 30s, attempt_deadline_at)`。manual/Reader/MCP-server timeout 180 秒，automatic timeout 120 秒。实例 worker 上限 8 由 worker pool 控制，数据库强制每用户有效 RUNNING 上限 2。

### 8.3 Heartbeat

`heartbeat(&ContentJobClaim) -> LeaseDeadline` 只做条件更新。owner/token/attempt/status 不匹配、lease 已过期或 attempt deadline 已到一律返回 `LeaseLost`；不复活旧 lease，不修改 attempt 编号。

### 8.4 Failure 与 retry

`complete_failure(claim, AttemptFailure)` 在 `User → Job → Attempt` 锁顺序下：

- 验证 claim 仍有效；写入 bounded metrics、metadata、error code、retryable 和 outcome_unknown。
- permanent failure 或当前 attempt 已达 max：attempt FAILED，job FAILED，清空 lease/deadline，设置 completed_at。
- retryable 且有剩余 attempt：attempt FAILED，job RETRY_WAIT，清空 lease/deadline，设置 `next_attempt_at`。
- 默认 backoff：attempt 1 后 5 秒，attempt 2 后 30 秒；provider `Retry-After` 取较大值但 clamp 到 1 小时。v1 max attempts 固定 3。
- unknown outcome 只记录事实，是否 retry 由稳定错误分类决定；不得把它写成 success 或删除历史成本。

### 8.5 Success 与 artifact 原子事务

`complete_success(claim, ArtifactCandidate, AttemptUsage) -> StoredArtifactResult`：

1. 在事务外完成 guest output schema、安全和 canonical JSON 验证；仓储仍重复检查 identity、size 和 canonical bytes。
2. 开启事务，锁 User → Job → Attempt，验证 claim 与 job snapshot。
3. 查询 `(user_id, identity_hash)`：
   - 不存在：插入 immutable artifact，`producer_job_id = current job`；
   - 存在且 identity columns 全等：复用；
   - hash 相同但 identity 不同：`HashCollision` 并 rollback。
4. 插入 `content_job_results`，完成 attempt SUCCEEDED，完成 job SUCCEEDED 并清空 lease。
5. 四步同一事务 commit；任一步失败全部 rollback。旧 worker、重复 completion 或 unique race不能生成第二个终态副作用。

### 8.6 Read contract

- `get_job(user_id, job_id)`、`list_attempts(user_id, job_id)`、`get_result(user_id, job_id)` 均先 tenant scope；跨用户与不存在统一 NotFound。
- `find_artifact_by_identity(user_id, identity)` 返回 current exact match。
- `list_entry_artifacts(user_id, entry_id, kind, locale)` 只返回用户可见 entry 的历史 artifacts，按 `created_at DESC, id DESC`。
- stale 不落库：caller 以当前 expected identity 与 stored identity 比较。这样 config/provider/plugin 变化无需批量更新旧行。

## 9. 三数据库并发与锁顺序

统一锁顺序：

```text
User → Job → Attempt → Artifact → JobResult
```

- enqueue 只需要 User → existing Job/Artifact → Result。
- claim、heartbeat、failure、success 不得先锁 job 再锁 user。
- PostgreSQL 使用 `clock_timestamp()`；MySQL 使用 `UTC_TIMESTAMP(6)`；SQLite 使用数据库 `CURRENT_TIMESTAMP` 不足微秒，仓储使用 `STRFTIME('%Y-%m-%d %H:%M:%f','now')` 的等价 SQL 并按现有 time codec 读取。
- PostgreSQL/MySQL 使用 `FOR UPDATE`；SQLite 用 no-op UPDATE 建立 writer lock，所有判断随后在同一 transaction 内重读。
- 不依赖部分唯一索引、NULL unique 语义、数据库 enum、JSON 原生类型或 backend-specific upsert 作为正确性前提。
- unique violation 只能作为最后防线；仓储必须在锁内查询并比较完整语义，不能把所有 unique error 模糊映射为 existing。

## 10. Tenant isolation 与敏感数据

- 所有 public repository read/write 都显式携带 user ID 或从不可伪造 claim 中取得 user ID。
- entry visibility 由 subscription 关系验证，不因 admin 角色绕过；管理员不隐式获得普通用户正文/artifact。
- provider secret、endpoint credential、request header、system prompt、完整 entry 文本、provider raw response、MCP credential/raw result、stack trace 不进入四张表。
- error code 最大 64 ASCII；worker message 不持久化。
- `execution_metadata_json` 与 `provenance_json` 只允许 schema-approved keys，未知 key fail closed；最大 32 KiB。
- payload 是用户内容，API 层仍须授权与安全渲染；数据库日志不得输出 payload/metadata/provenance 正文。

## 11. 错误合同

稳定 repository error kind：

```text
InvalidInput
NotFound
EntryChanged
IdempotencyConflict
HashCollision
NoWork
UserConcurrencyLimited
LeaseLost
AlreadyCompleted
AttemptsExhausted
ArtifactTooLarge
NonCanonicalJson
CorruptData
Database
```

数据库 driver error 不直接穿透 API。跨用户资源和不存在统一 `NotFound`。重复成功 completion 返回 `AlreadyCompleted`，不得静默覆盖计量；精确 idempotent enqueue 则返回 existing job。

## 12. 精确测试矩阵

### 12.1 Migration/entity

- migrate/rollback 创建并按反向依赖顺序删除四张表。
- 所有列、FK、unique/index 在 SQLite contract 中存在。
- PostgreSQL/MySQL CI 运行同一 schema smoke。
- entity round-trip 验证 nullable locale、UTC microseconds、bigint、boolean 和 JSON text。

### 12.2 Hash/model

- framed hash 防拼接歧义；domain separation 产生不同 hash。
- JSON key 顺序不同得到相同 canonical hash；值不同得到不同 hash。
- locale、ID、hash、enum、版本、大小、attempt/timeout 上限 validation。
- idempotency key 大小写产生不同 hash；空、控制字符、超过 255 bytes 拒绝。

### 12.3 Enqueue

- 可见 entry 入队 QUEUED；不可见/不存在/disabled user 都 NotFound。
- entry content hash 漂移返回 EntryChanged 且零写入。
- 同 key 同 request 返回同 job；同 key 不同 request 冲突。
- 并发同 key 最终一行。
- 完整 identity 已有 artifact 时创建零 attempt SUCCEEDED job + reused result。
- identity hash collision fixture fail closed。

### 12.4 Claim/recovery

- due ordering 稳定；future retry 不 claim。
- claim 创建 attempt 1、token 1、deadline/lease；heartbeat 续租但不越 deadline。
- 两个 worker 竞争同 job 只有一个 claim。
- 每用户第三个有效 RUNNING job 不 claim，其他用户仍可 claim。
- expired attempt 标 ABANDONED 并创建下一 attempt/token。
- 第三 attempt 过期后 job FAILED，不创建 attempt 4。
- stale owner/token、expired lease/deadline 的 heartbeat/completion 全部 LeaseLost。

### 12.5 Failure/success/artifact

- permanent failure terminal；retryable attempt 1/2 分别 schedule 5s/30s。
- Retry-After 取较大值并上限 1h；unknown outcome 持久化。
- success 原子写 artifact/result/attempt/job；artifact payload 不改变 entry。
- 两个不同 job 同 identity success 只产生一个 artifact，两条 result link 中一条 reused。
- induced result/attempt/job update failure rollback artifact insert 和全部终态变化。
- oversized/noncanonical payload/provenance 在事务前拒绝。
- provider/config/plugin/prompt/schema/MCP/content 任一 provenance 变化得到新 identity，旧 artifact 保留。

### 12.6 Backend contract

以下 suite 在 SQLite 必跑，并在配置 `RAINDROP_TEST_POSTGRES_URL` / `RAINDROP_TEST_MYSQL_URL` 时对对应 backend 必跑：

```text
content_job_enqueue_contract
content_job_claim_contract
content_job_recovery_contract
content_job_terminal_contract
content_artifact_identity_contract
content_tenant_isolation_contract
```

## 13. 实施成功标准

- migration、entities、model/hash 与 repository contract 全部实现，三 backend 使用同一 public interface。
- 幂等 enqueue、claim/recovery/fencing、failure/retry、success/artifact transaction 有可复现并发与 rollback 测试。
- 全量 `cargo fmt --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cargo test --locked --all-features` 通过。
- 现有 RSS、API、Reader、provider core 测试无回归；live RSS smoke 仍由显式环境变量控制。
- 文档明确该 core 是未来 Wasm AI plugin、Reader、lifecycle 与 MCP 的唯一执行入口。

## 14. 内部评审记录

本规格由主 Agent 进行一次有界内部评审，结论为可实施，关键修正已直接合入：

- 用 `content_job_results` 消除 jobs/artifacts 循环 FK并表达 artifact reuse。
- 用 idempotency hash 保持 MySQL collation 下的 opaque key case sensitivity，并规定 collision fail closed。
- 增加不可续期 `attempt_deadline_at`，避免 heartbeat 把 120/180 秒硬上限变成软上限。
- completion 强制同时检查活 lease 与 fencing token，避免暂停后的 zombie worker 提交。
- current/stale 改为相对 identity 的读时判断，避免批量更新 mutable artifact 状态。
- provider binding 使用完整 snapshot 而非只存 ID，确保 provider 修改后 provenance 可审计。
- 统一 User-first 锁顺序并让 SQLite writer serialization 对齐 PostgreSQL/MySQL 合同。

开放风险均属于后续切片：Wasm resource metering、provider quota reservation、MCP broker、lifecycle fan-out 与 API/UI，不阻塞本 core。
