# Raindrop 官方 AI 内容插件 v1 设计

日期：2026-07-17

状态：首版绑定设计

关联规格：`docs/superpowers/specs/2026-07-16-raindrop-design.md`

## 1. 决策摘要

Raindrop 首个稳定版本只交付一个内容插件：官方签名、随二进制打包的 Wasm Component，固定 `pluginKey = raindrop.ai-content`。它提供 `summarize` 和 `translate` 两个内容操作，必须通过与未来第三方插件相同的版本化 WIT ABI、manifest、capability broker、job、artifact 和 lifecycle host 运行；不得存在 native processor、route handler 直连模型或其它私有捷径。

首版对插件生态采取“合同开放、分发关闭”的边界：通用 plugin 表、manifest、WIT ABI、capability grant、五个 Feed 生命周期事件和契约 fixture 均为稳定接口，但产品不提供插件市场、远程安装、本地任意组件上传、custom JavaScript、多插件编排 UI 或第三方 SDK 发布承诺。以后增加插件、签名来源和安装 API 必须是 additive evolution，不能改变 `raindrop.ai-content` 已使用的 v1 合同。

AI 和 MCP 都是 host-brokered capability。插件永远拿不到 provider secret、MCP credential、transport、socket、数据库连接、文件、环境变量或进程。模型调用只经 `ai.generate_structured`，MCP 调用只经 `mcp.call_tool`；host 在调用时重新执行用户作用域授权、资源 allowlist、schema、预算和审计检查。

## 2. 目标与排除范围

### 2.1 首版目标

- 手动 API 和 Reader sidecar 可为用户可见条目创建摘要或翻译 job。
- 用户可显式配置 `feed.refresh.persisted` post-commit 自动触发；默认关闭，且必须指定 Feed、分类或明确的“全部已订阅 Feed”范围。
- 摘要与翻译默认不使用 MCP；用户可按操作启用只读 MCP context enrichment。
- Feed 抓取、解析、清洗和持久化即使插件、provider 或 MCP 故障也能独立完成，原文阅读永远可用。
- 所有执行具备用户作用域、稳定幂等 key、有界重试、审计记录、成本/资源上限和可重建 artifact。
- 五个生命周期入口的名称、输入、输出、失败语义和版本 fixture 在首版固定，为以后新增插件保留无破坏扩展路径。

### 2.2 首版明确不交付

- 插件 marketplace、catalog、评分、搜索、购买或自动更新服务。
- `POST /plugins/install`、远程 URL 安装、匿名安装、本地任意 Wasm 上传或未签名 sideload。
- custom JavaScript、WASI CLI、native dynamic library 或 shell 插件。
- 多插件排序、拖拽编排、条件图、可视化工作流或空壳 marketplace 页面。
- 允许插件直接访问任意网络、MCP transport、文件系统、环境变量、进程、数据库或 core repository。
- 首版 AI 插件调用具有写入、订阅变更、消息发送、支付、外部发布或其它副作用的 MCP 工具。首版没有可靠的逐次确认 UI，因此这类工具即使出现在连接 allowlist 中也由 broker 拒绝。
- 在同步 Feed hook、抓取事务或条目持久化事务中调用模型或 MCP。

## 3. 设计原则与信任边界

### 3.1 Contract-first

WIT package、manifest schema、配置 JSON Schema、lifecycle event schema、artifact schema、错误码和 HTTP DTO 先版本化，再实现 host 与组件。fixture 是合同的记录系统；Rust 类型、Wasm guest binding、OpenAPI 和前端类型从同一 schema 生成或由 CI 做双向一致性校验。

### 3.2 Additive evolution

- v1 已发布 record 只能增加可选字段，不能改字段含义、删除字段或把可选字段改为必填。
- 新 operation、event variant、capability 和 API endpoint 使用新标识追加；消费者遇到未知可选字段必须忽略。
- 不兼容 WIT 变更使用新 major package；manifest 和 persisted payload 保留明确 `schemaVersion`。
- 数据库遵循 expand → backfill → switch → later contract，破坏性删除至少跨一个稳定版本。

### 3.3 Least privilege

- 组件实例默认 capabilities 为空；manifest 的 requested capability 不等于授权。
- `raindrop.ai-content` 必须申请 `ai.generate_structured`；`mcp.call_tool` 是按用户、operation、connection、exact tool 动态授权的可选能力。
- Feed 文本、模型输出、MCP tool description、tool input 和 tool result 全部是不可信数据。
- secret 只存在 provider/MCP core domain 的加密存储和临时 transport 内存中，不进入 manifest、plugin config、WIT value、artifact、日志或错误正文。

## 4. 模块拓扑与执行流

```text
HTTP / Reader / Raindrop MCP server
               │ authenticated user + operation
               ▼
       Content Orchestrator ──────── idempotent content_jobs
               │                              │
               │                     leased Job Worker
               │                              │
               │                    Plugin Runtime Host
               │                    raindrop.ai-content.wasm
               │                       │             │
               │           ai.generate_structured   mcp.call_tool
               │                       │             │
               │                  Provider Core   MCP Client Core
               │                       │             │
               └──────────── validated artifact ◀────┘

Feed scheduler → fetch/parse/clean → short DB transaction
                                      ├─ entries/feed state
                                      └─ lifecycle_outbox
                                               │ post-commit, at least once
                                               ▼
                                      Lifecycle Dispatcher
                                               │ per eligible user
                                               ▼
                              plugin on-event → declarative job intents
                                               │
                                               └─ Content Orchestrator
```

关键不变量：

1. Feed transaction 只写 core 数据和 outbox，不实例化插件、不调用 provider、不调用 MCP。
2. `feed.refresh.persisted` payload 只含稳定 ID、计数和安全元数据，不含原始 Feed body、未清洗 HTML、URL query 或用户秘密。
3. lifecycle dispatcher 按用户订阅和插件配置 fan-out；插件只返回 declarative job intent，host 重新授权并幂等入队。
4. job worker 在事务外读取用户可见的 sanitized entry snapshot，并把该 snapshot 的 `contentHash` 固定到 job input。
5. artifact 落库是独立短事务；失败不会回滚 Feed 或改变原文。

## 5. 模块边界

| 模块 | 拥有的状态与职责 | 输入 | 输出 | 明确禁止 |
| --- | --- | --- | --- | --- |
| Feed core | 抓取、解析、清洗、条目标识、短事务、outbox 记录 | Feed claim、受限响应 | entries、refresh state、版本化 lifecycle event | provider/MCP 调用；把 raw body 放入 outbox |
| Lifecycle host | event schema、订阅匹配、lease、至少一次投递、per-user fan-out | committed outbox event、plugin manifest/config | event delivery、job intent | 在原事务执行；把别的用户数据传入组件 |
| Plugin registry | bundled component、manifest、digest/signature、版本与有效状态 | release artifact、DB installation row | 可实例化的已验证 component descriptor | 网络下载或任意本地上传 |
| Plugin runtime | Wasmtime store、WIT binding、fuel/内存/epoch deadline、capability broker | invocation envelope、显式 Feed/entry 数据 | event outcome 或 artifact candidate | ambient WASI、数据库/repository 访问、绕过 broker |
| Content orchestrator | authorization、job/idempotency、配置 snapshot、retry policy | 手动请求或 event job intent | leased job、状态和 execution record | 直接调用供应商 DTO；在 HTTP 请求中持有长事务 |
| Provider core | provider adapter、secret、quota、rate/token/cost 计量 | host 验证后的 structured generation request | schema-validated JSON、usage、provider-safe error | 将 secret 返回插件；接受未授权 provider ID |
| MCP client core | connection/transport/credential、tool inventory、策略、broker 调用 | host-issued tool binding、validated args、call context | bounded/redacted tool result、audit | 把 raw connection 或 credential 交给插件 |
| Artifact store | immutable derived result、current/stale 解析、provenance | validated plugin output、input/config/version hashes | user-scoped artifact | 覆盖 `entries.sanitized_content`；把 artifact 当记录系统 |
| Raindrop MCP server | 对外 resources/tools、token scope、用户隔离 | MCP request、call-chain metadata | 复用 content orchestrator 的 job/artifact DTO | 直接进入 plugin/DB；同链自调用 |

## 6. 官方组件的打包、发现与启用

### 6.1 发行物

发布构建把 `raindrop.ai-content.wasm` 嵌入二进制，并同时嵌入 canonical manifest、component SHA-256、签名和 release signing key ID。CI 在打包前验证：

- component digest 与 manifest 一致；
- 签名覆盖 canonical manifest 和 component digest；
- `pluginKey` 精确为 `raindrop.ai-content`；
- ABI major 为 host 支持的 `raindrop:content-plugin@1`；
- manifest 未声明 ambient WASI 权限；
- operation、hook、config schema 和 artifact schema fixture 全部存在。

启动迁移幂等创建唯一 installation row。`distribution = BUNDLED_OFFICIAL`、component path 不落用户可写目录、digest 不允许通过 API 修改。二进制升级时，host 在一个事务中写入新 manifest/version/digest；正在执行的 job 继续使用其记录的 `pluginVersion` 和 `componentDigest`，新 job 使用新版本。

实例管理员可全局 disable 官方插件；普通用户只能在全局 enabled 时启停自己的配置。卸载不属于首版 API，禁用不会删除历史 jobs/artifacts/audit。

### 6.2 Canonical manifest

```json
{
  "manifestVersion": 1,
  "pluginKey": "raindrop.ai-content",
  "version": "1.0.0",
  "abi": "raindrop:content-plugin@1.0.0",
  "distribution": "BUNDLED_OFFICIAL",
  "operations": ["summarize", "translate"],
  "lifecycleSubscriptions": [
    { "event": "feed.refresh.persisted", "schemaVersion": 1 }
  ],
  "capabilities": {
    "required": ["ai.generate_structured"],
    "optional": ["mcp.call_tool"]
  },
  "ambientPermissions": [],
  "configSchema": "raindrop://schemas/plugins/raindrop.ai-content/config/v1",
  "artifactSchemas": [
    "raindrop://schemas/artifacts/ai-summary/v1",
    "raindrop://schemas/artifacts/ai-translation/v1"
  ],
  "componentDigest": {
    "algorithm": "SHA-256",
    "valueEncoding": "LOWER_HEX",
    "valueSource": "RELEASE_BUILD"
  },
  "signature": {
    "algorithm": "Ed25519",
    "valueEncoding": "BASE64URL",
    "keyId": "raindrop-release-2026",
    "valueSource": "RELEASE_BUILD"
  }
}
```

release build 必须把 `valueSource` 替换为真实 `value`；带 `valueSource` 的模板 manifest 不能进入发行二进制或 installation row。签名覆盖移除 `signature.value` 后的 canonical manifest 与 component digest。

## 7. WIT ABI v1

### 7.1 Package 与 world

稳定 package 为 `raindrop:content-plugin@1.0.0`，world 为 `content-plugin-v1`。host 总是实现 capability interface，但每次调用仍依据当前 invocation grant 返回允许或 `CAPABILITY_DENIED`；因此配置变化不需要重新链接 component。

```wit
package raindrop:content-plugin@1.0.0;

world content-plugin-v1 {
  import host-ai;
  import host-mcp;
  export content-plugin;
}
```

guest export 只有三个稳定入口：

```text
content-plugin.descriptor() -> plugin-descriptor
content-plugin.execute(operation-request) -> result<artifact-candidate, plugin-error>
content-plugin.on-event(lifecycle-event) -> result<event-outcome, plugin-error>
```

`descriptor` 必须与 signed manifest 的 key/version/operations/hooks 一致，否则 host 拒绝实例化。guest 不导出网络、CLI 或数据库入口。

### 7.2 Operation request

host 构造且 guest 不得选择的上下文：

- `invocationId`、`jobId`、`idempotencyKey`；
- `pluginKey`、`pluginVersion`、`componentDigest`；
- `userScope` 的 opaque subject，不包含邮箱、token 或角色列表；
- `trigger = MANUAL_API | READER_SIDECAR | FEED_REFRESH_PERSISTED | MCP_SERVER`；
- `entryRef`、`contentHash`、sanitized title/text、canonical URL 的无 credential 形式、source locale；
- operation-specific config snapshot 和 `configHash`；
- host-issued provider binding；
- 当前 invocation 可见的 MCP tool bindings；
- `callChainId`、`remainingDepth`、deadline 和各项剩余预算。

组件可返回的 `artifact-candidate` 只含 schema ID、locale、canonical JSON payload 和非敏感 provenance hints。host 在落库前重新做 schema、大小、URL/Markdown 安全和 user/job/input hash 校验。

### 7.3 `ai.generate_structured`

稳定 capability 名为 `ai.generate_structured`，WIT function 为 `host-ai.generate-structured`。请求包含：

- host-issued `providerBindingId`，不能提交任意 provider URL/model/secret；
- `operation`、system instruction、显式标记为 untrusted 的 input JSON；
- output JSON Schema 和 schema ID；
- provider request ordinal；
- requested input/output token ceiling，不得高于 invocation 剩余预算。

host 执行：当前用户/provider 授权、provider enabled、quota reservation、rate/concurrency、token/cost estimate、timeout、adapter 调用、结构化 JSON parse 和 schema 校验。返回值只有 validated JSON、finish reason、token usage、可公开 model label 和 cost estimate；没有 request header、credential 或原始 provider transport。

组件不能要求 provider 执行 tool call。需要 MCP enrichment 时，组件先用一次受限 structured generation 产生符合 schema 的 tool plan，再调用 `mcp.call_tool`，最后把 tool result 作为 untrusted context 交给新的 `ai.generate_structured` 生成 artifact。host 同时限制 provider request 次数和 MCP call 次数，组件不能实现无界 agent loop。

### 7.4 `mcp.call_tool`

稳定 capability 名为 `mcp.call_tool`，WIT function 为 `host-mcp.call-tool`。请求只含：

- 当前 invocation 中的 host-issued `toolBindingId`；
- tool input schema 对应的 canonical JSON arguments；
- 单次 requested timeout，不得超过 host ceiling。

`toolBindingId` 映射到当前用户、AI plugin config、operation、connection ID 和 exact tool name。插件拿不到 MCP credential、endpoint credential-bearing URL、transport object、stdio command、raw socket 或 connection pool。host 每次调用重新检查 binding 未撤销、connection enabled、tool schema 未漂移、effect classification 和剩余预算。

返回值只含 schema-validated、bounded、secret-redacted JSON result，以及 connection/tool 的非秘密 display label。tool description 和 result 在传给模型时都放入 untrusted data 区域，不能改变 system policy、capability grant 或 output schema。

## 8. Feed 生命周期合同

### 8.1 稳定事件集合

| 事件 | 时点 | v1 输入 | 允许输出 | 投递语义 |
| --- | --- | --- | --- | --- |
| `feed.refresh.before` | 网络请求前 | refresh/feed ID、安全 URL metadata、条件请求状态 | continue、host-validated skip/retry hint、safe header patch | 同步；fixture 保留，官方 AI 插件不订阅 |
| `feed.refresh.fetched` | 响应大小限制后、解析前 | status、media type、bounded body handle、validator metadata | accept/reject、diagnostic、自定义 parser intent | 同步；fixture 保留，官方 AI 插件不订阅 |
| `entry.process` | 规范化/清洗后、持久化前 | host-computed identity inputs、sanitized entry | bounded content/annotation patch 或 drop reason | 同步；fixture 保留，官方 AI 插件不订阅 |
| `feed.refresh.persisted` | entries/feed/outbox 同事务提交后 | refresh/feed ID、new/updated entry refs、counts、content hashes | declarative content job intents | outbox 至少一次；官方 AI 插件唯一订阅事件 |
| `feed.refresh.completed` | 刷新终态 | success/not-modified/error code、counts、duration、安全诊断 | audit/notification intents | outbox 至少一次；fixture 保留，官方 AI 插件不订阅 |

所有事件 envelope 都含 `eventId`、`eventType`、`schemaVersion`、`refreshId`、`sequence`、`occurredAt` 和 `idempotencyKey`。`persisted` 使用 sequence 10 和 `refresh:{refreshId}:persisted:v1`；`completed` 使用 sequence 20 和 `refresh:{refreshId}:completed:v1`，与 RSS ingestion 合同一致。

### 8.2 同步 hook 安全限制

`before`、`fetched` 和 `entry.process` 是未来业务插件的同步 host contract，但首版没有可安装的第三方组件。即使以后启用，这三个入口也不能获得 `ai.generate_structured` 或 `mcp.call_tool`，不能打开数据库事务，默认 `FAIL_OPEN`，且任何 patch 由 Feed core 重新验证。安全过滤类 fail-closed policy 需要未来独立设计和显式管理员决策，首版没有对应插件或 UI。

### 8.3 `feed.refresh.persisted` fan-out

dispatcher 先 claim outbox event，再查询订阅该 Feed 且有效启用 `raindrop.ai-content` 自动规则的用户。每个用户收到独立的、最小化 event view；不会出现其他用户 ID、分类或设置。

官方组件根据 config snapshot 返回每个 entry/operation 的 job intent。host 对 intent 重新检查条目可见性、operation enabled、provider binding、自动范围和 quota，然后使用：

```text
event:{eventId}:plugin:raindrop.ai-content:user:{userId}:entry:{entryId}:op:{operation}:config:{configHash}
```

作为 job idempotency key。唯一约束保证 outbox 重投、dispatcher 崩溃重启或 guest 重复返回不会生成重复 job。

## 9. 摘要与翻译操作

### 9.1 `summarize`

输入是 host 选定的 sanitized title/text snapshot、source locale 和用户 style 配置。输出 schema `ai-summary/v1`：

```json
{
  "schemaVersion": 1,
  "sourceLanguage": "en",
  "summary": "...",
  "bullets": ["..."],
  "conclusion": null
}
```

`summary` 必填；`bullets` 最多 8 项；所有字符串受 artifact 总大小限制；不允许 raw HTML。引用 URL 必须来自 host 提供的 source 或 MCP result 中经过 URL policy 验证的 `http/https` URL。

### 9.2 `translate`

输入额外包含 BCP 47 `targetLocale`。输出 schema `ai-translation/v1`：

```json
{
  "schemaVersion": 1,
  "detectedSourceLanguage": "en",
  "targetLocale": "zh-CN",
  "title": "...",
  "bodyMarkdown": "..."
}
```

Markdown 禁止 raw HTML、data URL 和脚本协议；渲染仍经过前端 Markdown sanitizer。代码块、链接目标和无法可靠翻译的专有名词保持可追溯。翻译只生成 artifact，不改 `entries.title`、`entries.summary` 或 `entries.sanitized_content`。

### 9.3 Artifact current/stale

artifact identity 由 user、entry、operation/kind、target locale、`contentHash`、plugin key/version、provider/model label、prompt/schema version、`configHash` 和 MCP provenance hash 构成。相同 identity 直接复用成功 artifact；条目内容、配置、plugin/provider/prompt 版本变化后旧 artifact 变为 stale，但保留审计和手动查看能力。

## 10. MCP context enrichment

### 10.1 默认与授权交集

摘要和翻译的 MCP mode 默认 `DISABLED`。启用后，有效工具集合是以下条件的交集：

1. 当前用户拥有并启用了 connection；
2. 用户的 AI plugin config 对当前 operation 启用了 MCP；
3. config 明确列出 connection ID 和 exact tool name；
4. connection policy 也 allowlist 该 exact tool；
5. host tool inventory schema 与 config binding 一致；
6. tool effect classification 为 `READ_ONLY`；
7. 当前 trigger policy 允许该工具；
8. invocation 尚有 tool-call、depth、time、token 和 cost 预算。

任一条件不满足都返回稳定拒绝码，不做模糊 fallback 到同 connection 的其它 tool。tool 自报“只读”不构成信任；管理员/连接 policy 必须把工具标记为 `READ_ONLY`，`UNKNOWN` 按有副作用处理。

### 10.2 自动与手动流程

- `FEED_REFRESH_PERSISTED` 自动 job：只允许显式 allowlist 的 `READ_ONLY` 工具，最多 2 次 tool call。
- `MANUAL_API`、`READER_SIDECAR` 和 `MCP_SERVER` 手动 job：首版同样只允许 `READ_ONLY` 工具，最多 4 次 tool call。
- 具有副作用或 effect unknown 的工具需要逐次交互确认和 policy approval。首版不交付可靠确认 UI，因此 broker 一律返回 `MCP_SIDE_EFFECT_CONFIRMATION_REQUIRED`，不能用静态配置绕过。

### 10.3 Failure policy

每个 operation 配置 `FAIL_OPEN` 或 `FAIL_CLOSED`：

- `FAIL_OPEN`：MCP discovery/call/schema/timeout 失败后记录 degraded execution，清除失败工具结果，使用原始 Feed snapshot 继续一次无工具生成。授权拒绝、递归拒绝和预算耗尽也不会转为尝试其它工具。
- `FAIL_CLOSED`：job 以明确 MCP error 结束，不生成新的 artifact；已有 artifact 和原文继续可读。

两种策略都不能改变 Feed refresh/outbox 的成功状态，也不能阻塞 Reader 原文、已读或收藏操作。

### 10.4 限制与审计

host 对 MCP 强制：

- arguments canonical JSON 最大 64 KiB；result 进入 plugin 前最大 256 KiB；超限立即终止该 call；
- 单 call 默认 10 秒、硬上限 15 秒；每用户最多 2 个并发 AI-plugin MCP call，实例默认 16 个；
- 自动 job 最多 2 次、手动 job 最多 4 次；guest 无权提高；
- tool plan、arguments 和 result 分别做 JSON Schema 校验；schema 漂移使 binding 失效；
- tool description、arguments、result 在审计前执行 secret detector 和 connection-defined redaction；审计保存 bounded redacted JSON、原始值 digest、schema digest、connection/tool ID、duration、status 和 error code；
- 审计表按用户授权查询，日志只记录 audit ID 和非秘密 label，不复制完整 result。

### 10.5 与双向 MCP 的复用和递归防护

AI plugin 不实现 MCP transport，始终调用 `src/mcp` 的 client domain service。Raindrop 作为 MCP server 暴露的 `summarize_entry` / `translate_entry` 也只调用 content orchestrator，不直接实例化 plugin，从而复用相同 authorization、job、quota 和 artifact 语义。

每个手动、自动或 MCP server 发起的内容操作都创建或继承 `callChainId`，并携带 `originInstanceId`、已访问的 `(serverIdentity, toolName)` 集合和 `remainingDepth`。AI-plugin outbound MCP call：

- 默认 depth budget 为 2，每次 broker call 减 1；为 0 返回 `MCP_RECURSION_BLOCKED`；
- 目标解析为当前 Raindrop instance 时一律禁止调用 `summarize_entry`、`translate_entry` 或其它会再次进入同一 content orchestrator 的工具；
- 同一 chain 已出现相同 `(serverIdentity, toolName)` 时拒绝；
- Raindrop-to-Raindrop Streamable HTTP 在受保护 metadata 中传播 chain context；不支持传播的第三方 server 仍受本地 tool-call、deadline 和 concurrency 硬限制。

## 11. 数据模型

以下是逻辑字段；具体 SQL 类型继续遵循三数据库 migration 合同。

### 11.1 Plugin 与配置

- `plugin_installations(id, plugin_key, version, abi_version, distribution, component_digest, manifest_json, signature_key_id, signature, system_state, installed_at, updated_at)`；唯一 `plugin_key`。首版只有 `raindrop.ai-content`，`distribution = BUNDLED_OFFICIAL`。
- `plugin_configs(id, plugin_id, owner_user_id, schema_version, config_json, config_hash, is_enabled, revision, created_at, updated_at)`；唯一 `(plugin_id, owner_user_id)`。config 不含 secret。
- `plugin_capability_grants(id, plugin_id, owner_user_id, capability, operation, resource_type, resource_id, constraints_json, revision, created_at, revoked_at)`；MCP grant 精确到 connection/tool，AI grant 精确到 provider binding。
- `plugin_kv(plugin_id, owner_user_id, key, value, updated_at)` 作为通用 ABI 预留并有容量配额；官方 AI 插件 v1 manifest 不申请 scoped KV，因此不会写该表。

### 11.2 Job、execution 与 artifact

- `content_jobs(id, user_id, entry_id, operation, target_locale, trigger, plugin_key, plugin_version, component_digest, provider_binding_id, input_hash, config_hash, idempotency_key, call_chain_id, remaining_depth, status, attempts, next_attempt_at, created_at, completed_at)`；唯一 `(user_id, idempotency_key)`。
- `content_job_attempts(id, job_id, attempt, started_at, completed_at, status, error_code, retryable, provider_request_count, mcp_call_count, input_tokens, output_tokens, estimated_cost_micros, execution_metadata_json)`；唯一 `(job_id, attempt)`。
- `content_artifacts(id, user_id, entry_id, job_id, kind, locale, schema_id, input_hash, config_hash, processor_key, processor_version, provider_label, payload_json, provenance_json, created_at)`；唯一键覆盖完整 artifact identity。
- `mcp_tool_calls(id, job_attempt_id, user_id, call_chain_id, connection_id, tool_name, tool_schema_digest, arguments_redacted_json, arguments_digest, result_redacted_json, result_digest, status, error_code, duration_ms, created_at)`。

### 11.3 Lifecycle

- `lifecycle_outbox` 继续使用 RSS ingestion 已固定的 event type、refresh ID、sequence、payload version、canonical JSON、idempotency key、lease 和 `PENDING/DELIVERING/DELIVERED/DEAD` 状态。
- `lifecycle_deliveries(event_id, plugin_id, owner_user_id, config_hash, status, attempts, last_error_code, completed_at)`；唯一 `(event_id, plugin_id, owner_user_id, config_hash)`。

数据库是 job/artifact/audit 的记录系统；Wasmtime 内存、provider response stream 和 MCP transport state 都是短期状态，不可作为恢复依据。

## 12. 配置 JSON Schema

配置通过 `PUT /api/v1/plugins/raindrop.ai-content/config` 全量替换，必须带当前 revision 的 `If-Match`。host 先用 manifest 内 canonical schema 验证，再验证 provider/Feed/category/connection/tool 引用和用户所有权。配置不允许 secret、endpoint 或 raw header。

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "raindrop://schemas/plugins/raindrop.ai-content/config/v1",
  "type": "object",
  "additionalProperties": false,
  "required": ["schemaVersion", "operations", "automatic"],
  "properties": {
    "schemaVersion": { "const": 1 },
    "operations": {
      "type": "object",
      "additionalProperties": false,
      "required": ["summarize", "translate"],
      "properties": {
        "summarize": { "$ref": "#/$defs/summarizeOperation" },
        "translate": { "$ref": "#/$defs/translateOperation" }
      }
    },
    "automatic": { "$ref": "#/$defs/automatic" }
  },
  "$defs": {
    "mcp": {
      "type": "object",
      "additionalProperties": false,
      "required": ["mode", "failurePolicy", "maxToolCalls", "tools"],
      "properties": {
        "mode": { "enum": ["DISABLED", "CONTEXT_ENRICHMENT"] },
        "failurePolicy": { "enum": ["FAIL_OPEN", "FAIL_CLOSED"] },
        "maxToolCalls": { "type": "integer", "minimum": 0, "maximum": 4 },
        "tools": {
          "type": "array",
          "maxItems": 16,
          "uniqueItems": true,
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["connectionId", "toolName"],
            "properties": {
              "connectionId": { "type": "string", "minLength": 1, "maxLength": 64 },
              "toolName": { "type": "string", "minLength": 1, "maxLength": 128 }
            }
          }
        }
      }
    },
    "summarizeOperation": {
      "type": "object",
      "additionalProperties": false,
      "required": ["enabled", "providerId", "style", "maxOutputTokens", "mcp"],
      "properties": {
        "enabled": { "type": "boolean" },
        "providerId": { "type": "string", "minLength": 1, "maxLength": 64 },
        "style": { "enum": ["CONCISE", "BALANCED", "DETAILED"] },
        "maxOutputTokens": { "type": "integer", "minimum": 128, "maximum": 4096 },
        "mcp": { "$ref": "#/$defs/mcp" }
      }
    },
    "translateOperation": {
      "type": "object",
      "additionalProperties": false,
      "required": ["enabled", "providerId", "defaultTargetLocale", "maxOutputTokens", "mcp"],
      "properties": {
        "enabled": { "type": "boolean" },
        "providerId": { "type": "string", "minLength": 1, "maxLength": 64 },
        "defaultTargetLocale": { "type": "string", "minLength": 2, "maxLength": 35 },
        "maxOutputTokens": { "type": "integer", "minimum": 256, "maximum": 16384 },
        "mcp": { "$ref": "#/$defs/mcp" }
      }
    },
    "automatic": {
      "type": "object",
      "additionalProperties": false,
      "required": ["enabled", "operations", "allSubscribedFeeds", "feedIds", "categoryIds"],
      "properties": {
        "enabled": { "type": "boolean" },
        "operations": {
          "type": "array",
          "minItems": 1,
          "maxItems": 2,
          "uniqueItems": true,
          "items": { "enum": ["SUMMARIZE", "TRANSLATE"] }
        },
        "allSubscribedFeeds": { "type": "boolean" },
        "feedIds": {
          "type": "array",
          "maxItems": 1000,
          "uniqueItems": true,
          "items": { "type": "string", "minLength": 1, "maxLength": 64 }
        },
        "categoryIds": {
          "type": "array",
          "maxItems": 250,
          "uniqueItems": true,
          "items": { "type": "string", "minLength": 1, "maxLength": 64 }
        }
      }
    }
  }
}
```

跨字段验证规则：

- MCP `mode = DISABLED` 时 `maxToolCalls` 必须为 0 且 `tools` 必须为空；启用时 `maxToolCalls` 至少为 1。
- 自动触发 enabled 时，至少一个 operation 本身 enabled，且 `allSubscribedFeeds = true` 或 `feedIds/categoryIds` 至少一项非空。
- 自动翻译使用 translate operation 的 `defaultTargetLocale`；手动请求可覆盖 target locale，但覆盖值也进入 idempotency/artifact identity。
- 自动流程的有效 `maxToolCalls` 由配置值和 host ceiling 取较小值，最多 2。

## 13. HTTP API 与授权

首版 OpenAPI 只暴露官方插件的查询、状态、配置、执行记录和重试：

```text
GET    /api/v1/plugins
GET    /api/v1/plugins/raindrop.ai-content
PATCH  /api/v1/plugins/raindrop.ai-content/state
GET    /api/v1/plugins/raindrop.ai-content/config
PUT    /api/v1/plugins/raindrop.ai-content/config
GET    /api/v1/plugins/raindrop.ai-content/executions?cursor=&status=&operation=
GET    /api/v1/plugins/raindrop.ai-content/executions/:executionId
POST   /api/v1/plugins/raindrop.ai-content/executions/:executionId/retry

POST   /api/v1/entries/:entryId/summaries
POST   /api/v1/entries/:entryId/translations
GET    /api/v1/entries/:entryId/artifacts
```

`GET /plugins` 首版返回零或一个系统 descriptor；正常发行版始终为一个，实例全局禁用时仍返回但 state 为 disabled。API 不包含 install、uninstall、upload、catalog 或 marketplace link。未来增加这些能力必须新增 endpoint，不改变首版 DTO 的含义。

手动 operation 请求接受可选 `Idempotency-Key`；缺失时 server 生成，重复 key 在同一 user/entry/operation/target locale 下返回已有 job。成功入队返回 `202` 和 job URL。跨用户 entry、provider、connection、tool 或 execution ID 统一按资源不可见处理，不能通过错误差异枚举。

配置和执行记录默认只对所属用户可见。实例管理员管理共享 provider、MCP connection policy 和插件 system state，但 `ADMIN` 不隐式授予查看普通用户正文、artifact 或 MCP result 的权限。

## 14. UI 合同

Reader 继续以 Feed 原文为默认内容。摘要/翻译入口是正文工具栏和移动端次级 action sheet；结果出现在可关闭的 sidecar/inspector，不替换正文、不自动改变已读状态，也不阻断上一篇/下一篇。

状态必须区分：未配置、可执行、queued、running、degraded、succeeded、failed、stale。失败时保留原文和已有 artifact，展示安全错误码对应的本地化说明及可重试动作。

设置页只展示“官方 AI 内容插件”卡片：

- 官方签名/随应用提供的来源标识、版本和 digest 摘要；
- 用户启停、摘要/翻译 provider 与输出偏好；
- 自动触发范围和明确成本提示；
- MCP context enrichment 高级区域，默认关闭，只能从用户已有 connection 的 exact read-only tools 中选择；
- execution history、degraded/MCP call 标识和重试；
- 不显示 marketplace、安装按钮、上传 dropzone、第三方推荐、优先级排序或多插件画布。

桌面使用 ASTRYX 的 `Section`、`FormLayout`、`Selector`、`Banner`、`Table` 和 `AlertDialog`；移动端保持 44×44 px 操作目标、单任务 Reader 和现有返回/滚动恢复规则。设置变更不使用动画表达安全状态；MCP 开启和“全部已订阅 Feed”自动处理需要明确确认。

## 15. 错误合同与重试

### 15.1 稳定错误码

| Domain | 错误码 | Retry | 语义 |
| --- | --- | --- | --- |
| Plugin | `PLUGIN_DISABLED` | 否 | instance 或 user state disabled |
| Plugin | `PLUGIN_ABI_UNSUPPORTED` | 否 | WIT major 不受 host 支持 |
| Plugin | `PLUGIN_MANIFEST_INVALID` | 否 | manifest/descriptor/schema 不一致 |
| Plugin | `PLUGIN_SIGNATURE_INVALID` | 否 | bundled component 验签失败，启动时 quarantine |
| Plugin | `PLUGIN_CAPABILITY_DENIED` | 否 | 当前 invocation 没有 capability grant |
| Plugin | `PLUGIN_CONFIG_INVALID` | 否 | JSON Schema 或资源引用验证失败 |
| Plugin | `PLUGIN_FUEL_EXHAUSTED` | 有界 | guest 超出 fuel |
| Plugin | `PLUGIN_MEMORY_LIMIT` | 否 | linear memory 超限 |
| Plugin | `PLUGIN_TIMEOUT` | 有界 | component deadline 超限 |
| Plugin | `PLUGIN_OUTPUT_TOO_LARGE` | 否 | WIT/artifact 输出超限 |
| Plugin | `PLUGIN_TRAP` | 有界 | guest trap，脱敏记录 |
| AI | `AI_PROVIDER_NOT_CONFIGURED` | 否 | binding 缺失或 disabled |
| AI | `AI_QUOTA_EXCEEDED` | 否/到期后 | 用户/provider quota 不足 |
| AI | `AI_RATE_LIMITED` | 是 | 按 retry-after 有界重试 |
| AI | `AI_UPSTREAM_TIMEOUT` | 是 | provider timeout |
| AI | `AI_OUTPUT_SCHEMA_INVALID` | 最多一次 | 结构化输出不符合 schema |
| AI | `AI_COST_LIMIT_EXCEEDED` | 否 | 预留或累计成本超 invocation ceiling |
| MCP | `MCP_DISABLED` | 否 | operation 未启用 enrichment |
| MCP | `MCP_CONNECTION_DENIED` | 否 | user/connection grant 失败 |
| MCP | `MCP_TOOL_DENIED` | 否 | exact tool 不在交集 allowlist |
| MCP | `MCP_SIDE_EFFECT_CONFIRMATION_REQUIRED` | 否 | 非只读或 unknown tool，首版不支持 |
| MCP | `MCP_SCHEMA_INVALID` | 否 | input/result/tool schema 漂移或非法 |
| MCP | `MCP_TIMEOUT` | 是 | tool deadline 超限 |
| MCP | `MCP_RESULT_TOO_LARGE` | 否 | bounded result 超限 |
| MCP | `MCP_CALL_BUDGET_EXHAUSTED` | 否 | call count/time/token/cost budget 用尽 |
| MCP | `MCP_RECURSION_BLOCKED` | 否 | depth 或同链目标检查拒绝 |
| Job | `JOB_ALREADY_COMPLETED` | 否 | retry 已成功 job |
| Job | `JOB_NOT_RETRYABLE` | 否 | permanent error |
| Artifact | `ARTIFACT_STALE` | 不适用 | 查询明确标记，不作为失败 |

API 同步 validation/auth/conflict 继续映射统一 `422/401/403/404/409` error envelope。provider/plugin/MCP 的异步错误写入 job attempt 并由 execution API 返回稳定 code、retryable 和 request ID；不返回 stack、prompt、raw Feed、provider body、MCP credential 或未脱敏 tool result。

### 15.2 Retry 与至少一次

- lifecycle outbox 和 delivery 是至少一次；唯一 delivery/job idempotency key 消除重复副作用。
- job worker 使用 lease 和 attempt number；崩溃后可重领，但 artifact insert 使用完整 identity 唯一约束。
- provider timeout 后无法保证上游没有计费；host 在调用前预留预算，使用 provider idempotency header（若支持），并在 unknown outcome 审计中标记可能重复成本。
- permanent authorization/schema/limit 错误不自动重试。transient provider/MCP 错误使用有界指数退避；默认最多 3 个 job attempts。
- 用户 retry 创建新 attempt 或在配置/input/version 改变时创建新 job；不能篡改历史 attempt。

## 16. 资源与成本边界

| 边界 | 默认/硬上限 |
| --- | --- |
| Wasm linear memory | 64 MiB hard limit per instance |
| Guest fuel | 50,000,000 units per `execute`；5,000,000 per lifecycle callback |
| Guest CPU/epoch | 2 秒纯 guest 时间；host await 分别受 capability timeout 控制 |
| WIT request | 1 MiB；其中 sanitized entry text 最多 512 KiB，超出时 deterministic truncation 并记录 `truncated=true` |
| Artifact candidate | 512 KiB canonical JSON |
| Job wall time | 手动 180 秒、自动 120 秒硬上限 |
| Provider calls | 每 job 最多 3 次；单次 90 秒 |
| Provider tokens | summarize output 最多 4,096；translate output 最多 16,384；输入与输出还受 provider/user quota |
| Estimated cost | 已知价格 provider 默认每 job 250,000 micros USD-equivalent，管理员可下调；未知价格仍强制 token/request quota |
| MCP | 自动最多 2 calls，手动最多 4；单 call 15 秒、result 256 KiB、depth 2 |
| Concurrency | 每用户 2 个 AI jobs；实例 worker 默认 8；AI-plugin MCP 每用户 2、实例 16 |
| Audit | bounded redacted JSON；retention 由实例策略配置，删除 artifact 不删除仍在 retention 内的安全审计 |

host 可以通过管理员配置降低 ceiling，不能高于 binary hard limit。预算值和实际 usage 都写 execution record；guest 看到的只有剩余预算，不能修改 hard limit。

## 17. 版本、兼容与弃用策略

- Manifest schema 从 `manifestVersion = 1` 开始。未知顶层可选字段忽略；未知 required capability 或未知 manifest major 拒绝加载。
- WIT identity 为 `raindrop:content-plugin@1.x`。minor 只增加可选 record 字段、可选 capability 或新 operation/event variant；guest 必须通过 feature/capability negotiation 使用。
- Event payload `schemaVersion = 1` 与 event type 分别版本化。host 保留 canonical v1 fixture；新字段必须可缺省，重试旧 outbox payload 时不得需要新字段。
- Config 和 artifact schema 独立版本化。host 在读时迁移 config copy，不原地静默改变用户语义；artifact 永不改写，只通过新 artifact supersede。
- 新 WIT major 发布后，host 至少在两个后续稳定版本继续支持旧 major，并在 execution/admin UI 发 deprecation warning。删除旧 major 需要 release note、迁移检查和无活跃 installation/job 的证明。
- bundled official component 与 host 同版本发布，但仍必须通过 ABI compatibility suite；“同仓库”不是跳过合同测试的理由。

## 18. 测试矩阵

| 层 | 必测合同 |
| --- | --- |
| Release/package | digest/signature 成功；篡改 component/manifest 失败；唯一 plugin key；无 ambient WASI imports |
| Manifest/config | canonical serialization；JSON Schema 正反例；资源引用越权；revision/If-Match 冲突；config 无 secret |
| WIT ABI | guest binding round-trip；unsupported major；unknown optional field；descriptor/manifest mismatch；host capability denied |
| Lifecycle fixtures | 五个 event v1 fixture parse/round-trip；before/fetched/entry patch 再验证；persisted/completed sequence 与 key；payload 64 KiB/脱敏 |
| Transaction boundary | provider/MCP mock 断言 Feed persist transaction 内调用次数为 0；commit rollback 无 event；commit 后才 dispatch |
| At-least-once | duplicate outbox、delivery lease expiry、worker crash、重复 guest intent 最终只有一个 job/artifact |
| Operations | summarize/translate schema；manual/Reader/MCP-server trigger；content/config/version 变更使旧 artifact stale |
| Provider | secret 不进入 ABI/log/error；quota/rate/token/cost；timeout；invalid JSON/schema；provider idempotency header |
| Sandbox | 文件、环境、进程、数据库、socket、真实时钟/随机等未授权 import 拒绝；fuel、memory、epoch、output 限制 |
| MCP authorization | 默认关闭；用户 + operation + connection + exact tool 交集；撤销后立即拒绝；schema drift invalidates binding |
| MCP safety | background 只读；unknown/side-effect 拒绝；args/result limits；恶意 description/result；secret redaction；并发/call budget |
| MCP failure | fail-open 回退无工具生成且标 degraded；fail-closed job error；两者都不改变 Feed/Reader 原文 |
| MCP recursion | same-instance summarize/translate self-call；重复 target/tool；depth 0；Raindrop peer chain propagation |
| Repository | SQLite/PostgreSQL/MySQL 运行相同 job/idempotency/artifact/lifecycle/audit contract suite |
| API/auth | 只列官方插件；install/catalog route 不存在；跨用户 entry/config/execution/tool 不可枚举；retry permanent error 拒绝 |
| UI/E2E | 原文默认；sidecar queued/success/degraded/failure/stale；移动端返回/滚动；设置页无 marketplace/安装/编排入口 |

契约 fixture 建议固定在 `tests/fixtures/plugin-contract/v1/`，包含 manifest、config、五类 lifecycle event、两类 artifact 和错误 envelope。CI 同时由 Rust host 和 guest binding consumer 读取，防止只验证一侧。

## 19. 未来 additive 扩展步骤

1. 先发布 guest SDK、签名与兼容性测试工具，但仍不开放安装；用第二个内部 fixture component 证明 ABI 与官方插件无特权。
2. 增加管理员本地 signed package sideload，新增 package/install API 和权限 diff；默认仍拒绝未知签名根。
3. 在来源验证、撤销、升级回滚和安全响应成熟后增加受信 catalog。marketplace UI 只在后端分发和政策真实可用时出现。
4. 增加多插件 lifecycle delivery 时，沿用稳定 `(event, plugin, user, config)` delivery key；排序/编排作为新 capability 和新 UI，不修改 v1 单插件语义。
5. 增加副作用 MCP tool 前，先交付逐次确认 UI、可取消 pending approval、过期时间和审核测试；然后以新 policy 状态 additive 开启，不能把现有 `READ_ONLY` 自动授权重新解释。
6. 新内容 operation、新 provider、新 MCP transport 和新 artifact schema 分别追加 adapter/variant/schema；不得让插件获得 transport 或 secret。

## 20. 首版完成标准

- 发行二进制只包含并发现 `raindrop.ai-content` 一个官方签名 Wasm Component，篡改时 fail closed/quarantine；不存在 native shortcut。
- 手动 API、Reader sidecar 和 Raindrop MCP server 可通过相同 content orchestrator 创建 summarize/translate job，返回 user-scoped artifact。
- 配置开启后，`feed.refresh.persisted` 通过 outbox 至少一次 fan-out 并幂等排队；provider/MCP 在 Feed transaction 内的调用数严格为 0。
- 五个 lifecycle event、manifest、WIT、config、artifact 和 error v1 fixture 均通过 host/guest contract tests；官方插件只订阅 post-commit persisted。
- secret 不穿过 ABI；sandbox 无文件、环境、进程、数据库和任意网络；fuel/内存/时间/输出/token/cost ceiling 均有拒绝测试。
- MCP 默认关闭；启用后按用户、operation、connection 和 exact tool 取授权交集；自动任务只读，副作用/unknown tool 首版拒绝。
- MCP fail-open/fail-closed、schema/大小/超时/并发/call/depth 限制、脱敏审计和递归阻断均有测试。
- artifact 版本化且不覆盖 Feed 原文；AI/plugin/MCP 失败不影响 RSS 入库、原文阅读、已读或收藏。
- API/OpenAPI 只有官方插件的查询、启停、配置、execution 与 retry，没有 install/catalog/marketplace；设置页无空壳生态入口。
- SQLite、PostgreSQL、MySQL 通过相同 job、artifact、outbox、delivery 和 audit repository contract。

## 21. 自审结论

- 范围：首版交付一个真实官方插件，不声称开放第三方生态或市场。
- 边界：core、plugin、provider、job、artifact、lifecycle 和 MCP client/server 的输入、输出、所有权与禁止项明确。
- 事务：所有模型/MCP 工作均在 post-commit job 中执行，RSS 记录系统不依赖派生能力。
- 安全：Feed、模型和 MCP 内容均按不可信数据处理，secret 与 transport 不越过 host broker。
- 演进：manifest/WIT/event/config/artifact/API 都有版本和 additive/弃用规则，未来安装和多插件能力不需要破坏 v1。
- 可验证性：关键绑定决策都有 fixture、集成测试或三数据库 contract 对应，所有设计项均已绑定。
