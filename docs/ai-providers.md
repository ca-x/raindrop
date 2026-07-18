# AI Provider Core 运维合同

本文记录当前已经实现的 AI provider 内部核心。它包含三数据库记录、独立密钥环、四类非流式协议 adapter、SSRF-safe HTTPS transport 和统一 `ProviderClient`，但尚未提供 provider 管理 API/UI、内容 job/artifact、摘要/翻译入口、官方 Wasm 插件或 MCP。没有这些上层能力时，配置密钥不会自动发起模型请求。

## 支持的协议

| Provider kind | 默认 endpoint | Adapter path |
| --- | --- | --- |
| Anthropic Messages-compatible | `https://api.anthropic.com/` | `/v1/messages` |
| OpenAI Responses | `https://api.openai.com/` | `/v1/responses` |
| OpenAI Chat Completions-compatible | `https://api.openai.com/` | `/v1/chat/completions` |
| Google Gemini | `https://generativelanguage.googleapis.com/` | `/v1beta/models/{encodedModel}:generateContent` |

`providerKind` 在记录创建后不可修改。兼容服务可以设置带固定 path prefix 的自定义 HTTPS endpoint，例如 `https://gateway.example/tenant-a/`；adapter path 会安全地追加到该 prefix 后。endpoint 不接受 HTTP、userinfo、query、fragment、反斜杠、点路径、编码的点或路径分隔符，也不接受私有或特殊用途 literal IP。

## Provider secret keyring

Provider credential 使用独立密钥环加密，绝不从 session secret 派生。环境变量格式为：

```text
RAINDROP_PROVIDER_SECRET_KEYS=primary:<BASE64URL_32_BYTES>,previous:<BASE64URL_32_BYTES>
```

TOML 等价格式为：

```toml
provider_secret_keys = [
  "primary:<BASE64URL_32_BYTES>",
  "previous:<BASE64URL_32_BYTES>",
]
```

环境变量一旦存在，会替换完整 TOML 列表，不会逐项合并。每个 key ID 必须为 1 到 32 个 ASCII 字节，以字母或数字开头，且只能包含字母、数字、 `_` 和 `-`。key material 必须是无 padding 的 URL-safe base64，解码后正好 32 字节。重复 ID 或使用不同 ID 包装同一份 key material 都会使配置失败。

可以用下面的命令生成一份新 key material。命令输出本身就是秘密，应直接写入 secret manager 或权限受控的配置，不要粘贴到工单、日志或聊天记录：

```bash
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n'
```

配置是可选的，因此不使用 AI 的现有安装仍可启动。当前版本尚未把 provider repository/client 接入 HTTP runtime；未配置 keyring 时 RSS、Reader 和其他现有功能不受影响，但无法创建或解密 provider credential。后续 provider 管理服务启用时，应把这种状态暴露为 provider 功能不可用，而不是生成临时密钥或退化为明文。

## 轮换

列表第一项是 active encryption key；后续项只用于解密旧 envelope。标准轮换步骤：

1. 生成新的 32 字节 key，并使用新的唯一 key ID。
2. 把新 entry 放到列表首位，旧 active key 移到后面。
3. 更新所有实例的 secret 配置并重启。
4. 新建 provider 或重新提交 credential 时，会使用新的首项加密。
5. 在所有旧 envelope 已重包，或所有相关 provider credential 已重新录入前，保留旧 key。
6. 确认数据库中不再依赖旧 key 后，才能从列表删除它。

当前 core 没有批量 rewrap 命令。过早删除旧 key 会使对应 credential 无法解密；把旧 key 重新放回列表可以恢复。如果 key material 已永久丢失，AES-GCM ciphertext 无法恢复，只能从上游 provider 重新签发或重新录入 credential。

## 备份与恢复

数据库备份只包含 `rdsec1.<key-id>.<nonce>.<ciphertext+tag>`，不包含 key material。可恢复备份必须同时满足：

- 数据库、provider keyring 和配置版本属于同一恢复点；
- keyring 单独存放在受控 secret backup 中，不与公开数据库快照打包；
- 旧 key 在仍有 envelope 引用时继续保留；
- 恢复演练验证 provider binding 可以解密，但不打印 credential、envelope 或 endpoint。

如果只恢复数据库而没有对应 keyring，普通 RSS 数据仍可使用，但 provider credential 不可恢复。后续管理 UI/API 接通后，可通过重新输入 provider credential 生成新 envelope；不要编辑 `encrypted_secret` 或尝试从 session secret 派生替代 key。

## 加密与记录边界

- 算法：ring AES-256-GCM。
- nonce：每次加密独立生成 96-bit 随机值。
- AAD：绑定 envelope 版本、provider ID 和不可变 provider kind。
- credential：1 到 8,192 个 UTF-8 字节，只在 keyring、repository、adapter 和 transport 的窄边界内出现。
- `ai_providers` 是 SQLite、PostgreSQL 和 MySQL 的唯一记录系统；capability、quota 和 cost policy 使用规范化列。
- repository 使用显式 instance/user scope、用户隔离、enabled 检查和 revision CAS。

Credential、完整 endpoint/path、model、请求、响应、prompt、schema 和模型输出不会进入 metadata、binding、transport/call error 的 `Debug`/`Display`。`ProviderEndpoint` 自身的诊断输出只保留 scheme、canonical host、port 和 path segment 数量，不显示 path、query 或 credential。非 2xx response body 不读取、不保留；reqwest source 会先移除 URL。

## HTTPS transport 保证

生产 transport 只接受已经验证的 `ProviderEndpoint` 和 adapter 生成的 `EncodedProviderRequest`，并执行一次 POST：

- DNS：3 秒；解析 A/AAAA，最多接受 16 个原始答案。
- SSRF：任一答案为 private、loopback、link-local、documentation、multicast、transition-special 或其他 denied 地址时，整组拒绝。
- DNS pinning：批准的 socket 直接交给 reqwest `resolve_to_addrs`，TLS 仍校验原 hostname；连接后的 peer 必须精确属于批准集合。
- Proxy/redirect：禁用环境代理与自动重定向；所有 3xx 直接拒绝且不读取 body。
- 请求头：拒绝 `Host`、`Content-Length`、hop-by-hop 和 proxy headers；secret header 标记为 sensitive。
- 超时：connect 5 秒、first byte/header 20 秒、body idle 10 秒、total 90 秒。
- 响应：只支持单一 `identity`、`gzip`、`br` 或 zlib-wrapped `deflate`；compressed 与 decoded 各限 2 MiB，解压倍率最多 100x。
- Retry metadata：只接受一个合法 `Retry-After`，统一转换为 UTC deadline。

这些限制比 Feed GET 更严格，因为 provider POST 携带 credential，且没有安全的跨 authority redirect 用例。

## Client 与 policy 状态

`ProviderClient` 强制请求 model 与 binding model 完全一致，并拒绝超过 provider `max_output_tokens_per_request` 的请求。四种 adapter 都通过同一 transport 和 canonical structured-response decoder，模型输出始终按不可信数据处理。

`max_concurrency`、`requests_per_minute`、input token ceiling、cost rate 和 per-request cost ceiling 已持久化并经过跨数据库校验，但当前 provider client 不充当 scheduler。它们必须由后续 content job worker 在 reservation、执行和重试边界强制执行；在此之前不能把这些字段描述成已经生效的运行时额度。

Streaming 在 v1 明确不支持，`supports_streaming = true` 会被拒绝。

## 正式开放前的剩余门禁

确定性测试不会向真实 provider 发送请求。provider 管理 API/UI 对用户开放前，release 流程仍需使用专用低权限 credential，对四种受支持协议或实际启用的兼容 endpoint 运行最小 structured-generation contract probe，并确认 credential、请求和响应不会进入日志或 CI artifact。

以下仍是后续工作，不属于本 core 的完成状态：

- provider 管理 API、管理员/用户 scope 授权和 ASTRYX 设置 UI；
- `content_jobs`、artifact、缓存、额度 reservation、有界重试、摘要和翻译；
- 官方签名 `raindrop.ai-content` Wasm Component、WIT ABI、capability host 和 Reader sidecar；
- AI 插件通过 broker 调用的 MCP client，以及 Raindrop 自身的 MCP server。
