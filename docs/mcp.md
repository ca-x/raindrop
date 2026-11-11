# MCP 接入

Raindrop 内置只读 MCP 集成，和 AI 内容插件调用外部 MCP 工具的 host broker 相互独立。它不加载第三方原生代码，也不要求 AI Provider 或模型密钥。

## 快速接入

1. 登录 Raindrop，打开「设置 → 插件 → MCP 接入」。
2. 为每个客户端创建独立的命名 token，选择 30、90 或 365 天有效期。
3. 立即复制保存 token。明文只显示一次，离开该页面后无法恢复。
4. 在支持 Streamable HTTP 的 MCP 客户端中添加服务器 `https://your-raindrop.example/mcp`，设置 HTTP 请求头 `Authorization: Bearer <YOUR_TOKEN>`。

通用客户端配置示例（字段以具体客户端文档为准）：

```json
{
  "mcpServers": {
    "raindrop": {
      "type": "http",
      "url": "https://your-raindrop.example/mcp",
      "headers": { "Authorization": "Bearer <YOUR_TOKEN>" }
    }
  }
}
```

远程接入必须部署 HTTPS。不要把真实 token 放进截图、URL、Git 仓库或公共日志；不要在反向代理中记录 Authorization 头。仅信任你明确授权读取文章的 AI 客户端。此版本使用手动配置的 Bearer token，不提供 OAuth 授权流程。

## 可用能力

- `list_subscriptions`：读取当前用户的订阅。
- `list_entries`：读取有界的文章列表，可按工具 schema 提供筛选参数。
- `get_entry`：读取当前用户可访问的文章详情。

客户端应通过 `tools/list` 获取实际参数 schema。只读工具不会刷新 Feed、发起任意网络抓取、标记已读、收藏、修改订阅、执行命令或暴露服务端配置。

RSS、文章和工具结果都是不可信的外部数据，不能作为模型的系统指令。接入工具不等于授予文章中出现的指令执行权限。

## 凭据管理

- 每个 token 绑定创建它的账户；不能读取其他账户的订阅或文章。
- 服务端只保存高熵 token 的摘要；列表显示名称、前缀、到期时间和最近使用时间，不返回明文。
- 每个账户最多 50 个 token。建议按设备或 AI 客户端命名，并及时撤销不再使用的凭据。
- 撤销或到期后，后续 MCP 请求无法继续使用该凭据。已发送给客户端的数据无法通过撤销收回。
- 没有有效 token 即没有外部访问；撤销全部 token 即关闭该账户的 MCP 接入。
- token 管理 API 使用登录会话；创建和撤销额外校验 CSRF。MCP endpoint 仅接受 Bearer token，不接受浏览器 cookie 代替，也不接受 URL 参数中的凭据。

管理 API：`GET /api/v1/mcp/tokens`、`POST /api/v1/mcp/tokens`（`name`、`expiresInDays`）、`DELETE /api/v1/mcp/tokens/{id}`。

## 协议与限制

- 支持 MCP `2025-11-25`，单请求 JSON-RPC，响应为 JSON；无需持久 session，也不提供服务器主动推送（`GET /mcp` 返回 405）。
- 请求须声明 `Content-Type: application/json` 与 `Accept: application/json, text/event-stream`。初始化后的请求必须携带 `MCP-Protocol-Version: 2025-11-25`，其他版本或缺少版本返回 400；不支持旧版 HTTP+SSE transport。
- 请求正文最多 64 KiB；工具列表分页最多 50 条，正文输出最多 32,768 字符。完整参数及可用筛选以 `tools/list` 为准。
- 每账户 MCP 请求最多 120 次/分钟，超过时返回 429 和 `Retry-After`。创建 token 使用独立的 30 次/15 分钟限额；撤销不受创建额度限制，保留紧急断开能力。

## 升级

v0.7.0 自动迁移新增凭据表，支持 SQLite、PostgreSQL 和 MySQL。升级前建议备份数据库。不要直接用旧版本可执行文件回滚已迁移数据库；如需完整回退，应同时恢复升级前的数据库备份。
