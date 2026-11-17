# 文章朗读（TTS）

`raindrop.tts` 是随应用编译的独立原生插件，Web 和原生桌面端共用协议、
配置校验和文章分段逻辑；不会替换 AI 摘要/翻译插件或更改它的 ABI。

## 配置和试听

在插件设置中开启文章朗读，填写兼容 OpenAI 的语音 API：

点击朗读后，当前文章的纯文本会逐段发送到你配置的语音服务，可能产生服务商费用。
不会自动合成其他文章。本地资料库无需 Raindrop 服务端，但不代表离线语音合成，
仍需连接配置的 TTS API。

- **API Base URL**：填写 API 根地址，例如 `https://tts.example.com` 或
  `https://tts.example.com/v1`，不要填写完整 `/audio/speech` 地址。
  仅接受 HTTPS 公网地址；拒绝重定向、私有地址和 DNS 解析到内网的目标。
- **API Key**：可填写服务所需的令牌；无鉴权服务可以不填。留空保留已保存的
  密钥，明确勾选清除才会删除。新密钥加密后存储，配置读取不会返回明文。
- **模型**：如 `tts-1`，以服务实际支持的模型 ID 为准。
- **音色**：由用户手动选择并保存。提供 Edge 云扬
  `zh-CN-YunyangNeural`、晓晓 `zh-CN-XiaoxiaoNeural` 和 OpenAI 音色示例，
  也可选择自定义并填写其他语音 ID。示例不代表所有服务都支持；不会按文章语言
  自动更换音色。OpenAI 的标准语音合成协议没有通用的音色目录接口。
- **语速**：0.25–4，实际能力取决于服务。

点击测试会用当前尚未保存的配置生成一段真实 MP3，可能产生服务商费用。
测试成功后保存配置。文章工具栏只显示朗读、暂停/继续和停止图标，含可访问名称；
触屏操作不依赖悬停。长文逐段合成；停止、切换文章、注销或离开视图会取消后续请求。
失败不会自动重新发送可能计费的请求。

语音 API 必须允许普通 API 客户端访问。如果 Cloudflare 等网关对 `/v1/*`
返回人机挑战 HTML，应在网关放行相应 API 路径；Raindrop 不会把挑战页当音频播放。

桌面端使用原生音频输出，不依赖浏览器、WebView 或外部播放器。无声卡或音频输出
不可用时会显示错误，仍可正常阅读。桌面资料库的朗读使用本地配置，保存在本地数据库。
也可在设置中明确选择远端，编辑该服务器账户的语音配置；这不会切换本地文章的
朗读配置，也不会把本地正文或密钥静默发送到另一台 Raindrop 服务器。

## 密钥与存储

配置按用户隔离，保存在 `tts_configs`，更新需要匹配当前 `revision`，防止多端
互相覆盖。API Key 使用现有 Provider 密钥环和独立的 TTS 加密用途。
SQLite/交互式设置会复用数据目录内的 `provider-secret.key`；托管部署可使用
`RAINDROP_PROVIDER_SECRET_KEYS`。密钥环不可用时不会退化为明文存储。
同步/备份数据库时应一并保留对应密钥环；密钥不写入音频 URL 或发布附件。

## HTTP API

所有路由均要求登录，PUT/POST 另需 `x-csrf-token`。响应禁止缓存。

| 请求 | 用途 |
| --- | --- |
| `GET /api/v1/plugins/tts` | 当前用户配置，含 `hasApiKey`、`keyringAvailable` 和 `revision` |
| `PUT /api/v1/plugins/tts` | 保存 `isEnabled/baseUrl/model/voice/speed/revision`；`apiKey` 可省略、为字符串或 null |
| `POST /api/v1/plugins/tts/test` | 同配置草稿加 `text`（最多 200 字符），返回 `audio/mpeg` |
| `POST /api/v1/plugins/tts/entries/{id}/plan` | 已授权文章的 `entryId/contentHash/chunks` |
| `POST /api/v1/plugins/tts/entries/{id}/speak` | 使用 `contentHash/chunkIndex` 合成一段音频 |

上游固定使用 `POST audio/speech`，JSON 字段为 `model`、`voice`、`input`、
`speed`、`response_format: "mp3"`。每段至多 600 个 Unicode 字符，整篇最多
200,000 字符；音频每次最多 2 MiB，整个上游调用最多 90 秒。每用户同时 1 个、
全局同时 4 个请求；每用户每小时最多 512 次执行请求。配置变更或文章内容变化后，
旧的朗读计划返回 `TTS_CONFLICT`，需重新开始。服务端重新读取文章并验证访问权限，
客户端不能通过分段请求替换成任意文本。

## 开发测试

```sh
cargo +1.94.0 test --locked -p raindrop-tts
cargo +1.94.0 test --locked --test tts_api
npm --prefix web run test:ci -- src/features/speech
```

真实服务测试是显式忽略的 opt-in 测试，不会在普通测试或 CI 中读取真实密钥：

```sh
RAINDROP_TTS_LIVE_URL=https://tts.example.com \
RAINDROP_TTS_LIVE_TOKEN_FILE=/private/path/token \
RAINDROP_TTS_LIVE_VOICE=zh-CN-YunyangNeural \
cargo +1.94.0 test --locked --test tts_live -- --ignored
```

可额外指定 `RAINDROP_TTS_LIVE_AUDIO_FILE` 保存测试音频。不要把真实密钥、
含密钥的配置或现场数据提交到仓库。
