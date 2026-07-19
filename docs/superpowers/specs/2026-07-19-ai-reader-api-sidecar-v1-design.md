# Raindrop AI Reader API and Sidecar v1 Design

日期：2026-07-19

状态：bounded 内部评审通过，Critical 0 / Important 0

关联规格：

- docs/superpowers/specs/2026-07-16-raindrop-design.md
- docs/superpowers/specs/2026-07-19-production-ai-bundle-runtime-v1-design.md
- web/DESIGN.md

## 1. 决策摘要

本切片把已经完成的 Provider、官方插件、Content Job、Artifact 和生产 worker 接到真实用户入口，形成第一个可用的 AI Reader：

1. 已登录用户可以创建和修改自己的 Anthropic Messages、OpenAI Responses、OpenAI Chat Completions、Google Gemini Provider。
2. 用户可以为官方 raindrop.ai-content 插件配置摘要和翻译。
3. Reader 可以显式发起摘要或翻译，查看排队、执行、重试等待、成功和失败状态。
4. 成功 artifact 以类型化数据展示，原始文章始终是默认内容，AI 失败不影响阅读。
5. worker 内部自动重试继续使用同一 job。用户手动重试创建新的 job，并重新捕获当前 entry、config 和 provider snapshot。

本切片不实现自动 Feed 生命周期入队、MCP transport、第三方插件管理、Provider 物理删除、流式输出或聊天。MCP 的 WIT/config/tool-plan 合同已经存在，但生产 processor 仍明确返回 MCP_UNAVAILABLE；界面只显示“合同已预留，transport 尚未连接”，不伪装为可用能力。

用户已把后续评审与确认委托给主 agent，因此本文件的一次 bounded DDIA/API/security/UI 自评等价于本切片批准，不再设置人工确认门。

## 2. 参考吸收与改进

CommaFeed 的 SettingsPage 把显示、通知、脚本和个人资料集中在同一入口，Reader 操作紧贴文章，并保持稳定的键盘导航。这三个方向保留。

Raindrop 对其做以下改进：

- 设置使用显式 Save，不在每次开关变化时立即写完整对象。
- API 使用严格增量合同和 revision，而不是把内部 settings 实体直接暴露给客户端。
- Provider credential 永不回读，空白 credential 在编辑时表示保持原值。
- AI output 不混入 Feed 正文持久化字段，不替换 sanitized original content。
- AI artifact 是可重建的派生状态，以完整 identity 判断 current，配置、模型、正文或插件版本变化后不会把旧结果冒充为最新结果。
- 摘要和翻译是 Reader 中安静、可关闭的 sidecar，不改变当前 entry、queue 顺序、scroll anchor 或 read state。

## 3. Assumptions and scope

- Rust edition 2024，MSRV 1.94，现有 Axum、SeaORM、Tokio、Wasmtime、ASTRYX 0.1.6 和 Lingui 版本保持不变。
- 不新增数据库表或 migration。现有 ai_providers、plugin_configs、content_jobs、content_job_attempts、content_artifacts 和 content_job_results 是记录系统。
- Provider secret keyring 只从 RAINDROP_PROVIDER_SECRET_KEYS 加载。没有 keyring 时不生成临时密钥。
- Instance Provider 对普通用户只读可见；v1 只允许用户创建和修改 USER scope Provider。
- v1 不提供 Provider 物理删除。Disable 是可逆的管理动作，避免 config 引用和历史 artifact provenance 被无声破坏。物理删除需要后续显式 reference graph 和审计合同。
- 手动操作是唯一入队来源。automatic config 始终写为 disabled。
- MCP config 始终写为 disabled。后续 transport 接入时通过 additive API 字段开放。
- 原文读取、已读/收藏、Feed refresh 和 Reader keyboard flow 不等待 AI 请求。
- 不新增 Rust crate 或 npm package。

## 4. Tech stack

- Rust 2024，Axum 0.8.9，SeaORM 1.1.19，SQLite/PostgreSQL/MySQL。
- 现有 ProviderSecretKeyring、ProviderRepository、PluginRegistryRepository、ContentRepository 和 ContentRuntimeHandle。
- React 19，TypeScript 7，Lingui 6.5，ASTRYX 0.1.6，Vite 8，Vitest 4，Playwright 1.61。
- ASTRYX TabList、Selector、CheckboxInput、TextInput、NumberInput、Collapsible、List、Item、Banner、ProgressBar、Markdown 和 Button。
- 单一样式策略保持现有 ASTRYX StyleX 组件加 Raindrop CSS token/layout layer，不引入第二套 CSS framework。

## 5. Record system and derived state

关系数据库仍是权威记录系统：

- ai_providers：Provider metadata、policy 和加密 credential。
- plugin_configs：用户级官方 AI 配置和 optimistic revision。
- content_jobs / attempts：异步执行状态、lease、fencing、重试和 usage。
- content_artifacts：按完整 identity 不可变保存的派生结果。
- content_job_results：job 到 artifact 的稳定关联。

Artifact identity 必须继续包含：

- user 和 entry；
- entry content hash 和 canonical invocation input hash；
- plugin key、version、component digest；
- config hash；
- provider id、kind、model、revision；
- prompt version、schema id；
- target locale 和 MCP provenance hash。

配置、Provider revision、entry content、插件版本、prompt/schema 或 target locale 任一变化都会形成新 identity。Overview 只返回当前 identity 的 job/artifact。旧 artifact 保留用于审计和去重，但 v1 不在 Reader 中显示为 current，也不自动删除。

数据库事务只覆盖短记录操作：

1. Provider create/update。
2. Plugin config replace。
3. Content enqueue、claim、heartbeat、terminal write。

Provider HTTP、Wasm、MCP 或模型执行不得进入数据库事务。Content runtime notify 是 commit 后的 best-effort liveness hint，数据库 due query 才是最终恢复机制。

## 6. Shared production composition

main 只解析一次 RAINDROP_PROVIDER_SECRET_KEYS：

~~~rust
Option<Arc<ProviderSecretKeyring>>
~~~

该对象同时传给：

- ProductionContentRuntime，用于 decrypt 和真实 Provider 调用；
- AppState，用于 Provider create 和 credential rotation API。

ProviderRepository 改为持有 Option<Arc<ProviderSecretKeyring>>。Metadata list/get 和不修改 credential 的 update 在 keyring 缺失时仍可工作；create、credential update 和 load_enabled_binding 在缺失时返回 SecretUnavailable。这样用户仍能看见并 disable 已存在的 Provider，API 不需要复制原始 SecretString 列表。

AppState 新增独立 provider_mutation_limiter 和 content_mutation_limiter，不复用 subscriptions、organization 或 preferences 的预算。

## 7. Provider domain and HTTP contract

### 7.1 Endpoints

~~~text
GET   /api/v1/ai/providers
POST  /api/v1/ai/providers
GET   /api/v1/ai/providers/:providerId
PATCH /api/v1/ai/providers/:providerId
~~~

所有响应使用 no-store / no-cache。GET 需要 CurrentUser。POST/PATCH 需要 CurrentUser、CsrfGuard 和 provider mutation limiter。

### 7.2 Provider response

~~~json
{
  "keyringStatus": "AVAILABLE",
  "items": [
    {
      "providerId": "00000000-0000-4000-8000-000000000101",
      "scope": "USER",
      "canEdit": true,
      "displayName": "Primary OpenAI",
      "kind": "OPENAI_RESPONSES",
      "endpoint": "https://api.openai.com/v1/",
      "model": "gpt-5-mini",
      "capabilities": {
        "supportsUsage": true,
        "supportsIdempotency": true
      },
      "policy": {
        "maxConcurrency": 2,
        "requestsPerMinute": 30,
        "maxInputTokensPerRequest": 128000,
        "maxOutputTokensPerRequest": 4096,
        "inputCostMicrosPerMillionTokens": null,
        "outputCostMicrosPerMillionTokens": null,
        "maxCostMicrosPerRequest": 250000
      },
      "isEnabled": true,
      "revision": 0,
      "createdAt": "2026-07-19T00:00:00.000000Z",
      "updatedAt": "2026-07-19T00:00:00.000000Z"
    }
  ]
}
~~~

keyringStatus 是 AVAILABLE 或 UNAVAILABLE。Credential、encrypted envelope、key id 和 raw config 永不返回。supportsStreaming 不进入 v1 wire contract，因为现有 processor 不消费 streaming。

GET detail 使用相同 item shape。Well-formed 但不可见的 Provider 返回 404。Instance Provider 可由 list/get 返回 scope INSTANCE 和 canEdit false；PATCH Instance Provider 返回同一 404，避免暴露写权限差异。

### 7.3 Create

POST 只创建当前用户 scope。Body：

~~~json
{
  "displayName": "Primary OpenAI",
  "kind": "OPENAI_RESPONSES",
  "endpoint": null,
  "model": "gpt-5-mini",
  "credential": "secret",
  "capabilities": {
    "supportsUsage": true,
    "supportsIdempotency": true
  },
  "policy": {
    "maxConcurrency": 2,
    "requestsPerMinute": 30,
    "maxInputTokensPerRequest": 128000,
    "maxOutputTokensPerRequest": 4096,
    "inputCostMicrosPerMillionTokens": null,
    "outputCostMicrosPerMillionTokens": null,
    "maxCostMicrosPerRequest": 250000
  },
  "isEnabled": true
}
~~~

endpoint null 使用 kind 的 canonical default。成功返回 201 和 Location。Credential 进入 SecretString 后不得 Debug、Serialize、log 或 field echo。

每个用户最多创建 32 个 USER scope Provider。达到上限后 POST 返回 409 PROVIDER_LIMIT_REACHED。该硬上限使 list 保持有界，因此 v1 不增加无意义的 Provider pagination；Instance Provider 由后续管理员切片控制，不消耗用户配额。

### 7.4 Patch

PATCH 要求 expectedRevision，并至少出现一个可变字段：

~~~json
{
  "expectedRevision": 0,
  "displayName": "Primary OpenAI",
  "credential": null,
  "isEnabled": false
}
~~~

credential omitted 或 null 都表示保持原值。非空字符串表示 rotation。Revision mismatch 返回 409。Provider domain validation 失败返回 422。Keyring 缺失只阻止 create 或非空 credential rotation；metadata/policy/disable patch 仍可提交。

## 8. Official AI config contract

### 8.1 Endpoints

~~~text
GET /api/v1/ai/config
PUT /api/v1/ai/config
~~~

GET 返回：

~~~json
{
  "pluginState": "READY",
  "mcpState": "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
  "config": null
}
~~~

config 为 null 表示用户尚未保存。Plugin installation 缺失、disabled 或 quarantined 时 pluginState 分别为 UNAVAILABLE、DISABLED 或 QUARANTINED。

已配置时：

~~~json
{
  "pluginState": "READY",
  "mcpState": "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
  "config": {
    "revision": 3,
    "isEnabled": true,
    "summary": {
      "enabled": true,
      "providerId": "00000000-0000-4000-8000-000000000101",
      "style": "BALANCED",
      "maxOutputTokens": 1024
    },
    "translation": {
      "enabled": true,
      "providerId": "00000000-0000-4000-8000-000000000101",
      "defaultTargetLocale": "zh-CN",
      "maxOutputTokens": 4096
    }
  }
}
~~~

PUT 是完整替换，body 与 config 相同并增加 expectedRevision。首次创建 expectedRevision 为 null，后续必须精确匹配。isEnabled 必须精确等于 summary.enabled || translation.enabled；enabled operation 的 Provider 必须对当前用户可见且 enabled。Provider kind 不限制 operation。

API 写入 canonical AiContentConfig：

- schemaVersion 1；
- summarize / translate 使用请求值；
- 两个 MCP block 固定 DISABLED、FAIL_OPEN、0 calls、空 tools；
- automatic 固定 disabled、operations 保留 summarize/translate、allSubscribedFeeds false、空 feed/category。

## 9. Content execution API

### 9.1 Entry overview

~~~text
GET /api/v1/entries/:entryId/ai?translationLocale=zh-CN
~~~

translationLocale 省略时使用 config.defaultTargetLocale。响应同时给出 summary 和 translation 当前 identity 状态：

~~~json
{
  "availability": "READY",
  "mcpState": "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
  "summary": {
    "operation": "SUMMARIZE",
    "state": "IDLE",
    "job": null,
    "artifact": null
  },
  "translation": {
    "operation": "TRANSLATE",
    "targetLocale": "zh-CN",
    "state": "SUCCEEDED",
    "job": {
      "jobId": "00000000-0000-4000-8000-000000000201",
      "status": "SUCCEEDED",
      "attempts": 1,
      "maxAttempts": 3,
      "nextAttemptAt": "2026-07-19T00:00:00.000000Z",
      "lastErrorCode": null,
      "createdAt": "2026-07-19T00:00:00.000000Z",
      "startedAt": "2026-07-19T00:00:01.000000Z",
      "completedAt": "2026-07-19T00:00:02.000000Z"
    },
    "artifact": {
      "artifactId": "00000000-0000-4000-8000-000000000301",
      "kind": "AI_TRANSLATION",
      "providerLabel": "gpt-5-mini",
      "createdAt": "2026-07-19T00:00:02.000000Z",
      "detectedSourceLanguage": "en",
      "targetLocale": "zh-CN",
      "title": "标题",
      "bodyMarkdown": "正文"
    }
  }
}
~~~

availability：

- READY：plugin/config/provider metadata 足以计算 identity。
- NOT_CONFIGURED：没有 config。
- DISABLED：plugin config 或 operation disabled。
- PROVIDER_UNAVAILABLE：Provider missing/disabled。
- PLUGIN_UNAVAILABLE：installation missing/disabled/quarantined。

Operation state 是 UNAVAILABLE、DISABLED、IDLE、QUEUED、RUNNING、RETRY_WAIT、SUCCEEDED 或 FAILED。旧 identity 的 job/artifact 不返回。

### 9.2 Enqueue

~~~text
POST /api/v1/entries/:entryId/ai/jobs
~~~

Body：

~~~json
{
  "operation": "SUMMARIZE",
  "targetLocale": null,
  "idempotencyKey": "reader:00000000-0000-4000-8000-000000000401"
}
~~~

SUMMARIZE 要求 targetLocale null；TRANSLATE 要求 canonical BCP-47 locale。服务在 enqueue 前：

1. 按 current user 加载 visible entry execution projection。
2. 加载官方 installation 和当前 user config。
3. 验证 operation enabled 和 Provider visible/enabled。
4. 构造 ContentInvocationInput 和完整 ArtifactIdentity。
5. 若 current artifact 已存在，可在 keyring 缺失时复用。
6. 若需要真实执行而 keyring 缺失，返回 503 AI_PROVIDER_KEYRING_UNAVAILABLE，不创建永远无法运行的 job。
7. 调用 ContentRepository::enqueue，commit 后 notify content runtime。

新 queued 或 reused job 返回 201 和 Location。相同 user idempotency key + 相同 request 返回 200 Existing 和同一 Location。相同 key + 不同 request 返回 409 IDEMPOTENCY_CONFLICT。

### 9.3 Job status and result

~~~text
GET /api/v1/ai/jobs/:jobId
GET /api/v1/ai/jobs/:jobId/result
~~~

两者按 CurrentUser 隔离。跨租户和不存在统一 404。

Status 返回 job fields。Result 只在 SUCCEEDED 时返回类型化 artifact；其他状态返回 409 AI_RESULT_NOT_READY。Raw payloadJson 和 provenanceJson 不进入 wire contract。API 在返回前再次使用 SummaryArtifact 或 TranslationArtifact 解析持久化 payload，损坏数据返回 redacted 500。

Summary artifact：

~~~json
{
  "artifactId": "uuid",
  "kind": "AI_SUMMARY",
  "providerLabel": "gpt-5-mini",
  "createdAt": "2026-07-19T00:00:02.000000Z",
  "sourceLanguage": "en",
  "summary": "Short summary",
  "bullets": ["Point one"],
  "conclusion": null
}
~~~

### 9.4 Manual retry

~~~text
POST /api/v1/ai/jobs/:jobId/retry
~~~

Body：

~~~json
{
  "idempotencyKey": "reader-retry:00000000-0000-4000-8000-000000000501"
}
~~~

只有 terminal FAILED job 可手动重试，其他状态返回 409 AI_JOB_NOT_RETRYABLE。Retry 不修改旧 job、attempt 或 artifact，不复用旧 identity。服务从旧 job 读取 operation 和 target locale，然后针对当前 entry/config/provider snapshot 走同一 enqueue service。这样正文、配置或模型变化后重试不会执行过期快照。

worker 内部 retry 仍受 maxAttempts 3 控制，并在同一 job 中使用 RETRY_WAIT。用户手动 retry 是新 job，也重新获得最多 3 次内部 attempt。

## 10. Shared operation contract

processor 私有常量移动到共享只读模块：

~~~rust
pub struct OfficialAiOperationContract {
    pub plugin_key: &'static str,
    pub prompt_version: &'static str,
    pub schema_id: &'static str,
}

pub const fn official_ai_contract(
    operation: ContentJobOperation,
) -> OfficialAiOperationContract;
~~~

Enqueue service 与 OfficialAiProcessor 必须使用同一函数，禁止复制 prompt/schema 字符串。Bundled plugin key/version/ABI 也使用公开只读常量，避免 API、worker 和 manifest identity 漂移。

ContentRepository 新增：

~~~rust
pub async fn get_execution_entry_for_user(
    &self,
    user_id: &str,
    entry_id: &str,
) -> Result<ContentExecutionEntry, ContentRepositoryError>;

pub async fn find_latest_job_by_identity(
    &self,
    user_id: &str,
    identity: &ArtifactIdentity,
) -> Result<Option<JobSnapshot>, ContentRepositoryError>;
~~~

第一个方法复用 execution_entry SQL 和 sanitized text extraction，不把 contentHash 加到公开 Reader DTO。第二个方法按 user + identity hash 查询并验证完整 identity，按 created_at DESC、id DESC 返回最新 job。

## 11. Error and cache contract

新增稳定 code：

| 条件 | HTTP | code |
| --- | ---: | --- |
| body/path/locale/provider/config 验证失败 | 422 | VALIDATION_ERROR |
| session 缺失或失效 | 401 | AUTHENTICATION_REQUIRED |
| CSRF 失败 | 403 | FORBIDDEN |
| user-scoped resource 不存在或不可见 | 404 | NOT_FOUND |
| revision mismatch | 409 | REVISION_CONFLICT |
| 用户 Provider 已达到 32 个 | 409 | PROVIDER_LIMIT_REACHED |
| idempotency key 已用于不同请求 | 409 | IDEMPOTENCY_CONFLICT |
| job 不能手动重试 | 409 | AI_JOB_NOT_RETRYABLE |
| result 尚未成功 | 409 | AI_RESULT_NOT_READY |
| keyring 未配置且需要 credential/执行 | 503 | AI_PROVIDER_KEYRING_UNAVAILABLE |
| plugin/config/provider 暂不可执行 | 409 | AI_UNAVAILABLE |
| mutation budget 耗尽 | 429 | RATE_LIMITED |
| DB、secret envelope、artifact 或 stored config 损坏 | 500 | INTERNAL_ERROR |

所有 AI route 的成功和失败都添加：

~~~http
Cache-Control: no-store
Pragma: no-cache
~~~

错误不包含 credential、endpoint、model、prompt、正文、payload、provenance、SQL、backend error 或 stack。

## 12. OpenAPI and generated frontend types

新增 committed artifacts：

~~~text
docs/openapi/ai-provider-v1.json
docs/openapi/ai-content-v1.json
~~~

生成：

~~~text
web/src/features/ai/api/provider.generated.ts
web/src/features/ai/api/content.generated.ts
~~~

web/scripts/generate-reader-types.mjs 注册两个 artifact。Handwritten API wrapper 只负责 request、CSRF、AbortSignal 和 generated runtime validator，不创建第二份 wire interface。

## 13. Settings UI

现有 Settings Dialog 使用 ASTRYX TabList 分为 Appearance 和 AI。PreferencesDialog 拆分为 focused appearance panel，AI 模块放在 web/src/features/ai/settings，避免一个大 TSX 文件。

AI panel 顺序：

1. Banner 显示 keyring 或 plugin unavailable 状态。
2. Provider List 使用 ASTRYX List/Item，显示 scope、kind、model、enabled 状态和 Edit。
3. Add Provider 和 Edit Provider 使用同一 inline Collapsible form，不打开 nested dialog。
4. 基础字段使用 TextInput、Selector、CheckboxInput。
5. Policy 放在 Advanced Collapsible，使用 NumberInput。
6. AI content config 放在 Provider 之后，Summary 和 Translation 使用 CheckboxInput、Provider Selector、SegmentedControl 和 locale Selector。
7. 页面底部一个 Save AI settings 按钮。Provider form 有自己的 Save/Cancel，因为 credential rotation 和 config revision 是独立资源。

CheckboxInput 用于需要显式保存的布尔字段。ASTRYX Switch 不使用，因为其语义是立即生效。

Credential edit field永远为空，description 明确“留空保持现有密钥”。UI 不显示掩码长度，不从服务端接收 secret sentinel。

MCP 区域只显示 info Banner 和 disabled status，不提供可点击但无效的开关。

## 14. Reader sidecar UX

ArticleToolbar 增加 Summary 和 Translate 两个次要动作。默认仍只显示原文。首次打开动作时加载 entry AI overview。

Sidecar 位于 ArticleToolbar 与 article scroll plane 之间：

- 默认关闭，不占文章空间。
- 打开后使用 TabList 切换 Summary/Translation。
- 桌面最大高度 320px，compact 最大高度 45dvh，内部内容独立滚动。
- 原文 article 节点始终保留在 DOM 中，scrollTop、focus heading 和 read/star state 不重置。
- IDLE 显示 Run summary / Translate。
- QUEUED、RUNNING、RETRY_WAIT 使用状态文本和 ProgressBar/Spinner，不伪造完成百分比。
- FAILED 显示稳定用户文案和 Retry。
- SUCCEEDED Summary 使用 Heading/Text/List，Translation 使用 ASTRYX Markdown。
- Markdown headingLevelStart 从 3 开始，链接只允许 http/https，禁止 innerHTML。
- 切换 entry 时 abort 请求和 poll，并加载新 entry 的 overview。旧 entry 的晚响应不得覆盖当前 sidecar。

如果 config 未完成，sidecar 显示 Open AI settings。它不自动入队，不自动打开，不将 AI result 写入原文，也不改变 queue selection。

## 15. Responsive and accessibility

- 1280x800：Reader 三栏保持现状，sidecar 在文章区内展开。
- 900x800：两区 Reader 不新增第四列，sidecar 仍在文章区内。
- 390x844 和 360x800：sidecar 最大 45dvh，原文仍可滚动，所有动作至少 44px。
- TabList、Collapsible、Selector、CheckboxInput、Button 和 Markdown 使用 ASTRYX 原生语义。
- 状态变化使用 aria-live polite；错误 Banner 不抢焦点。
- 打开 sidecar 后焦点移动到 sidecar heading，关闭时回到触发按钮。
- reduced motion 下不做空间移动。Routine polling、job state 和 keyboard navigation 不动画。
- 中文与英文都验证长 Provider 名、长 model、错误文案和按钮 label。

视觉论点：暖纸阅读面、墨蓝单强调色、无装饰性渐变。控制层使用系统 sans，原文和 AI 阅读结果保持 Charter 与显式 CJK serif fallback。AI panel 使用背景 lightness step 和 hairline divider，不堆叠通用 card。

## 16. Threat model

| Boundary | Abuse case | Control |
| --- | --- | --- |
| Provider credential body | Secret 被回显、日志化或缓存 | SecretString、redacted Debug、no-store、response allowlist、无 secret sentinel |
| Provider endpoint | SSRF、私网访问、userinfo/query trick | ProviderEndpoint HTTPS canonical validation、literal private rejection、现有 transport DNS/redirect policy |
| User ownership | 猜测 Provider/job/entry UUID 读取他人数据 | CurrentUser scope，cross-tenant 统一 404，owner 不来自 body |
| Mutation | CSRF 和请求洪泛 | CsrfGuard，独立 per-user limiter，64 KiB body limit |
| Idempotency | 网络重试重复付费 | user-scoped idempotency key + request hash + unique constraint |
| Model prompt | Feed 正文注入系统指令 | 正文只放 untrusted input，权限在 host 代码执行，prompt 不是安全边界 |
| Model output | XSS、危险 URL、任意命令 | Summary/Translation strict parser，safe Markdown，React/ASTRYX renderer，无 eval/innerHTML/shell/SQL |
| Cost | 大正文、大 token、循环重试 | 512 KiB execution text cap，provider token/cost policy，2 concurrent user jobs，3 attempts，operation timeout |
| MCP | UI 暗示未实现能力 | 明确 unavailable state，processor fail closed，v1 不接受 enabled MCP config |
| Stored corruption | raw payload 或 secret 泄漏 | strict decode，generic 500，Debug redaction，raw JSON 不进入 wire |

## 17. Commands

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 fmt --all -- --check
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_api
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_content_api
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_job_contracts
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_worker_processor
npm --prefix web run generate:reader-types
npm --prefix web run check:reader-types
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cd web && npx playwright test --project reader-1280x800 --project reader-900x800 --project reader-390x844 --project reader-360x800
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 clippy --locked --all-targets --all-features -- -D warnings
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features
git diff --check
~~~

## 18. Project structure

~~~text
src/api/ai/providers.rs                 Provider routes and wire mapping
src/api/ai/content.rs                   config, overview, job, result, retry routes
src/api/ai/mod.rs                       scoped AI router
src/content/ai/service.rs               current identity and enqueue/retry service
src/content/worker/contracts.rs         shared official prompt/schema contract
src/content/provider/repository.rs      optional shared keyring and visible metadata lookup
src/content/jobs/repository.rs          execution projection and latest identity job lookup
src/plugins/config.rs                    public typed config getters/style
src/app.rs                              shared keyring and dedicated limiters
src/main.rs / src/background.rs         one-time keyring composition
docs/openapi/ai-provider-v1.json        Provider public contract
docs/openapi/ai-content-v1.json         config/execution public contract
web/src/features/ai/api                 generated types and request wrappers
web/src/features/ai/model               settings and per-entry controllers
web/src/features/ai/settings            provider/config focused components
web/src/features/ai/reader              sidecar, summary, translation views
~~~

## 19. Code style

HTTP DTO、domain model 和 database entity 保持分离。Handler 只做 boundary validation、authorization、mapping 和 service call：

~~~rust
async fn enqueue_entry_ai(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(entry_id): Path<String>,
    ApiJson(request): ApiJson<EnqueueAiRequest>,
) -> Result<Response, ApiError> {
    state.content_mutation_limiter.check(&user.id)?;
    let outcome = ai_service(&state)?
        .enqueue(&user.id, &entry_id, request.into_command()?)
        .await
        .map_err(map_ai_error)?;
    Ok(outcome.into_response())
}
~~~

禁止把 route 变成通用 service locator，禁止把 raw JSON entity 当 public DTO，禁止在前端重复 generated wire types。

## 20. Testing strategy

- Provider repository：optional keyring、metadata-only read、create/rotation unavailable、user/instance visibility、revision、credential redaction。
- Provider API：201/Location、list/get、instance read-only、secret absence、unknown fields、CSRF、rate limit、no-store、cross-user 404。
- Config API：null default、create/replace revision、provider visibility/enabled validation、fixed MCP/automatic encoding、plugin unavailable。
- Content service：current identity completeness、shared prompt/schema constants、artifact reuse、keyring missing no dead job、idempotency conflict、current snapshot retry。
- Content API：overview states、job status、typed result、result not ready、manual retry、cross-tenant 404、no raw payload/provenance。
- Frontend unit：generated validator、provider secret never cached、draft preservation、revision conflict、entry-change abort、late response suppression、poll stop、safe Markdown link handling。
- Browser：real Provider/config setup against local production binary，stub provider transport for deterministic artifact，sidecar original-content persistence，desktop/mobile containment，theme/locale，zero console/page errors。
- Existing RSS、Reader、preferences、setup/auth、embedded web、worker 和 production runtime tests remain green。

## 21. Boundaries

### Always

- CurrentUser owns every Provider config/job/artifact operation。
- Mutation requires CsrfGuard and dedicated limiter。
- Credential never leaves the request-to-encryption path。
- Artifact output is untrusted until strict typed parsing succeeds。
- Original article stays available regardless of AI state。
- Committed OpenAPI is the only frontend wire DTO source。
- Main agent performs one bounded review and fresh verification before commit。

### Never

- No physical Provider delete in v1。
- No automatic lifecycle enqueue。
- No enabled MCP config or fake MCP success。
- No raw payloadJson/provenanceJson/credential in API。
- No job mutation for manual retry。
- No model call inside database transaction。
- No nested Settings dialog、custom generic controls、fourth Reader column or large monolithic TS file。
- No new dependency without a demonstrated missing capability。

## 22. Success criteria

- A user can create one of four Provider kinds，save summary/translation config，open a real Reader entry，run both operations，observe terminal status and read validated results。
- A second user cannot list、read、modify、retry or fetch the first user's Provider/job/artifact。
- Credential never appears in response、Debug、logs、frontend state snapshot、OpenAPI example or committed fixture。
- Missing keyring permits metadata read/disable but blocks new credentials and non-reused execution with explicit 503。
- Duplicate enqueue with the same idempotency key does not create or charge a second execution。
- Manual retry creates a new job from current snapshot and leaves the failed job unchanged。
- Config/provider/entry/plugin changes make old artifacts non-current without deleting them。
- Sidecar is closed by default，does not replace original content，preserves article scroll/focus，and fits 1280x800、900x800、390x844 and 360x800。
- Focused tests、web suite/build、full Rust suite、browser checks and CI are green before the slice is reported complete。

## 23. Internal self-review

- DDIA：record 与 derived state 边界清晰；完整 identity 和 immutable artifact 防止错误复用；manual retry 不改历史；notify 只是 liveness hint；无跨网络事务。
- API：端点为 additive v1 resource；输入输出有 typed schema；credential/raw JSON 不泄漏；revision、idempotency、404、409、503 语义明确；list 不需要 pagination，因为每用户 Provider 上限在 API validation 中固定为 32。
- Security：CurrentUser、CSRF、dedicated limiter、HTTPS endpoint validation、secret redaction、LLM output parsing、token/cost/attempt bounds 和 no-store 均有直接测试 seam。
- UI：ASTRYX first；Settings 无 nested dialog；CheckboxInput 匹配显式保存；sidecar 不新增第四列；原文优先；移动端有独立高度和 touch target 约束。
- Scope：Provider 删除、automatic lifecycle dispatcher、MCP transport、第三方插件管理、streaming 和聊天明确留在后续切片，不以 placeholder 假实现。
- Review findings：初稿曾考虑在 keyring 缺失时完全关闭 Provider GET，以及复用旧 identity 做 manual retry。前者会妨碍恢复/disable，后者会执行过期正文或模型。规格已改为 metadata-only read 和 current-snapshot retry。Critical 0，Important 0。
- Open questions：无阻塞问题，设计可进入 implementation plan。
