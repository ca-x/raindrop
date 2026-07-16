# RSS ingestion 安全与测试策略

日期：2026-07-16
范围：RSS core 的 URL 安全、抓取、解析、清洗、持久化与测试；不修改 `Cargo.toml` 或源代码。

## 1. 可直接采用的结论

1. 将抓取链路拆成 `normalize -> resolve and classify -> pinned fetch -> bounded decode -> quick-xml preflight -> feedparser-rs parse_with_limits -> normalize entries -> Raindrop ammonia sanitize -> idempotent persist`。网络、解析、清洗全部在数据库事务外完成。
2. 默认仅允许 HTTPS。若管理员显式开启 insecure HTTP，仍应用完全相同的 SSRF 控制；禁止 HTTPS 重定向降级到 HTTP。HTTP 与 HTTPS URL 不合并为同一 Feed。
3. 禁止 reqwest 自动重定向、ambient/system proxy 和自动解压。每个跳转手工解析并重新执行 URL、DNS、IP 和 scheme 校验，最多 5 跳。
4. DNS 必须只解析一次并把本次得到的已验证地址固定到本次连接；不能“先 lookup 校验、再让 HTTP client 自行 lookup”。任何一个解析结果属于拒绝范围时，整次抓取失败。
5. 首版公网策略是“只允许明确的全球单播地址”，不是仅列出三个 RFC1918 网段。IPv4、IPv6、IPv4-mapped IPv6、NAT64/transition 地址都要经过统一分类。
6. 默认预算：DNS 3 秒、TCP/TLS connect 5 秒、响应头/首包 10 秒、连续读空闲 10 秒、单跳 20 秒、含重定向整次刷新 30 秒；压缩传输体 2 MiB、解压后 10 MiB、压缩比 100:1、条目 5,000、XML 深度 128。
7. `Content-Type` 只是提示。XML/JSON Feed MIME 可直接尝试；`text/plain`、`text/html`、`application/octet-stream` 仅在 BOM/空白后 body sniff 命中 XML 或 JSON 时容错解析。
8. `ETag` 和 `Last-Modified` 原样保存和回发；validator 必须绑定到产生它的最终响应 URL，跨 origin 或最终 URL 改变时不转发旧 validator。304 是成功刷新。
9. `feedparser-rs 0.5.5` 只负责结构解析，且必须关闭默认 `http` feature；不使用其 `parse_url` 或通用 sanitizer helper。清洗策略统一由 Raindrop 显式配置的 `ammonia` 完成，避免抓取/清洗策略分散或版本升级时悄然变化。
10. 外链仅保留 `http/https`，渲染时固定 `rel="noopener noreferrer nofollow"` 和 `referrerpolicy="no-referrer"`。外链图片默认不自动加载、不由服务端抓取；用户显式加载时才允许浏览器直连。未来图片代理必须复用同一套 SSRF 与大小限制。
11. `(feed_id, identity_hash)` 唯一约束是最终仲裁；重复刷新执行 upsert，内容没变时不写正文，内容变化时更新同一条目并保留 `inserted_at`。二次刷新必须报告 `new_entries = 0`，而不是依赖调用方去重。
12. CI 使用合成、确定性 fixture 与本地 scripted server。IT之家测试标记为 `#[ignore]` 且只有 `RAINDROP_LIVE_RSS_SMOKE=1` 时运行，不成为普通 CI 或发布阻塞项。

## 2. 当前项目约束

- 当前工程尚无 `src/feeds`、Feed schema 或 RSS 依赖。
- `AppState` 已采用 Axum state 注入；后续应注入 `FeedService`/`FeedFetcher`，route handler 不直接解析 URL、抓取或写 SQL。
- API 已有 64 KiB 请求上限、统一 `ApiError`、`requestId` 和脱敏错误风格。订阅 URL 请求体仍应额外限制为 4,096 字节并使用 `deny_unknown_fields`。
- 浏览器变更请求应继续要求 `CurrentUser + CsrfGuard`；订阅、刷新和条目查询必须包含用户作用域。
- 当前 CSP 的 `img-src 'self' data:` 会阻止远程图片，这与“默认不自动加载远程图片”的策略一致。不要为了 RSS 正文直接放宽为任意 `https:`。
- URL 常含私有 token 或难以察觉的查询参数；日志只记录 `feed_id`、规范化 host、状态码和内部错误分类，不记录完整 URL/query。

## 3. 信任边界与威胁模型

| 边界 | 主要滥用 | 必须控制 |
| --- | --- | --- |
| 订阅 URL | SSRF、混淆 IP、凭据泄漏、超长输入 | 规范化、scheme/credential 拒绝、长度限制、解析后 IP 分类 |
| DNS -> socket | rebinding、多个 A/AAAA 中藏私网地址 | 校验全部结果、一次解析、连接固定、连接后 peer 二次核对 |
| redirect | 跳到 metadata/私网、HTTPS 降级、循环 | 禁止自动跳转、逐跳重验、5 跳、scheme downgrade 拒绝 |
| HTTP body | 慢速响应、无限 chunk、压缩炸弹 | 分阶段超时、压缩/解压双上限、比率上限、流式读取 |
| XML/JSON | DTD/实体、深度/数量 DoS、畸形编码 | DTD/ENTITY 拒绝、深度/事件/条目上限、无 panic |
| entry HTML | 存储型 XSS、跟踪图片、危险 URL | 服务端白名单清洗、远程图片默认关闭、CSP |
| persistence | GUID 复用、并发重复、部分提交 | 稳定 identity、唯一约束、短事务、并发幂等 |

## 4. URL normalization 与 HTTP/HTTPS 策略

### 4.1 输入规则

- 先拒绝长度大于 4,096 字节、NUL/C0 control、CR/LF 和仅空白输入；只 trim 首尾 ASCII whitespace。
- 用现有 `url` crate 解析 absolute URL；仅接受 `http`、`https`。
- 拒绝任何 username/password，即使 password 为空；拒绝 zone identifier、缺 host、非法端口。
- 规范结果使用小写 scheme/IDNA host、canonical IPv4/IPv6、移除 fragment、移除默认端口、空 path 变 `/`。保留 query 顺序和重复 key，不排序 query。
- DNS 分类必须使用 URL parser 已 canonicalize 的 host，覆盖十进制/八进制/十六进制 IPv4 混淆，例如 `2130706433`、`0177.0.0.1`、`0x7f000001`。
- 域名末尾单个 root dot 在 normalized identity 中移除；DNS 查询也使用同一 canonical host。
- `source_url` 可保存用户输入的无凭据形式用于显示；去重和抓取使用 `normalized_url`。不得在错误或日志中回显 query。

### 4.2 scheme 决策

- 默认配置：只接受 HTTPS。
- 管理员显式启用 `allow_insecure_http` 后可订阅 HTTP，UI/API 返回可识别警告；这不是用户可自行绕过的选项。
- HTTPS -> HTTP redirect 永远拒绝。HTTP -> HTTPS 允许；HTTP -> HTTP 仅在 insecure HTTP 已启用时允许。
- 不自动把 HTTP 改写为 HTTPS，因为会改变 Feed identity 和可达性；可在 UI 给出“尝试 HTTPS”的显式建议。

### 4.3 最少单元测试

- 大小写 host、IDNA、尾点、默认端口、空 path、dot segment、fragment、重复 query。
- userinfo、非 HTTP scheme、network-path reference、畸形 percent encoding、控制字符、超长 URL。
- 所有 IPv4 混淆写法和 `[::ffff:127.0.0.1]` 规范化后仍被拒绝。
- HTTP 配置开关与 HTTPS downgrade 矩阵。

## 5. DNS、rebinding、redirect 与地址拒绝

### 5.1 resolver/connector 契约

定义可注入的 `DnsResolver`，返回本次解析的全部 `IpAddr`。生产流程：

1. 解析 A 与 AAAA，限制最多 16 个地址，空集合失败，整个解析受 3 秒 timeout。
2. 对全部地址执行 `AddressPolicy::classify`；只要有一个 denied 地址就 fail closed，防止攻击者同时返回公网与私网地址。
3. 把同一组 approved `SocketAddr` 固定给本跳的 HTTP connector，同时保留原 host 用作 HTTP Host 与 TLS SNI。
4. HTTP client 不得再次 DNS lookup；禁用 system proxy。连接建立后若能取得 remote peer，再确认它属于 approved set。
5. client/连接池不能跨 DNS 验证周期复用。最简单且安全的首版实现是每个 redirect hop 使用短生命周期、redirect-disabled、host-to-address pinned client；后续只有在能证明 pinning 生命周期正确时再优化池化。

DNSSEC 不能替代上述控制；它验证 DNS 数据来源，不阻止域名所有者故意返回内网地址或 rebinding。

### 5.2 默认拒绝范围

策略应由固定 CIDR 表加 embedded IPv4 检查实现，并有表驱动测试。至少拒绝：

- IPv4：unspecified、`0.0.0.0/8`、RFC1918、`100.64.0.0/10`、loopback、link-local、IETF protocol assignment、documentation、benchmark、multicast、reserved、broadcast。
- IPv6：unspecified、loopback、IPv4-compatible/mapped 中嵌入的任何 denied IPv4、NAT64 well-known/local-use 中嵌入的 denied IPv4、ULA、link-local、documentation、benchmark/ORCHID、multicast、reserved/非全球单播。
- 云 metadata 无需只列单点：`169.254.0.0/16`、ULA/link-local、CGNAT 已整体拒绝，从而覆盖 `169.254.169.254`、`fd00:ec2::254`、`100.100.100.200` 等常见目标。
- 首版建议同时拒绝 6to4/Teredo 等 transition range，降低嵌入地址与平台路由差异造成的绕过；如出现真实兼容需求，再以测试和显式配置放行。

“允许内网 Feed”必须是管理员实例级风险开关，并使用独立 policy；即便启用，也仍拒绝 unspecified、multicast、metadata 和跨 scheme downgrade。不要把测试用 loopback 放行开关暴露为生产配置。

### 5.3 redirect

- reqwest redirect policy 固定为 none；手工处理 301/302/303/307/308。
- `Location` 可为 relative，使用当前响应 URL resolve，随后重新 normalization、scheme、DNS、IP 校验。
- 最多 5 跳，记录 normalized URL 集合检测循环。
- 不把 Authorization、Cookie、用户自定义敏感 header、旧 origin 的 validators 转发到新 origin。RSS client 默认不使用 cookie。
- 永久 redirect 只有在一次完整成功解析后才可更新 fetch target；订阅 identity 是否迁移另做显式事务，不能仅因 301 就覆盖原记录。

### 5.4 安全测试

- fake resolver 返回 public + private 混合集合：零连接。
- fake resolver 第一次 public、若第二次调用则 private：断言 resolver 仅被调用一次，connector 使用第一次 approved set。
- redirect 从 public host 指向 `127.0.0.1`、metadata、IPv6 ULA、十进制 IPv4：目标 server 零请求。
- redirect loop、6 跳、HTTPS downgrade、relative redirect、跨 origin validator/header 泄漏。

## 6. HTTP 请求、超时、大小与 Content-Type

### 6.1 默认 header

- 稳定、可联系但不泄漏实例信息的 `User-Agent: Raindrop/<version> (+project URL)`。
- `Accept: application/atom+xml, application/rss+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.9, application/json;q=0.8, text/plain;q=0.5, */*;q=0.1`。
- `Accept-Encoding: br, gzip, deflate`；不发送 cookie、referer 或浏览器 header。
- 同一 validator URL 才发送 `If-None-Match`/`If-Modified-Since`。ETag 作为 opaque header value 原样保存；非法 header value 不回发。

### 6.2 时间预算

| 阶段 | 默认 | 失败分类 |
| --- | ---: | --- |
| DNS | 3 s | transient DNS timeout |
| TCP + TLS | 5 s | transient connect/TLS；证书错误单独分类 |
| headers/first byte | 10 s | transient first-byte timeout |
| body idle | 10 s | transient read timeout |
| one hop | 20 s | transient hop timeout |
| whole refresh | 30 s | transient total timeout |

总预算是最外层 deadline，重定向不能重置它。测试使用 paused Tokio time 或毫秒级专用配置，不让 CI 真等数十秒。

### 6.3 body 与压缩

- 若 `Content-Length > 2 MiB`，在读取前拒绝；chunked/缺失长度仍在 stream 中累计 compressed bytes。
- 禁用 reqwest 自动解压，按 Content-Encoding 只支持 `br/gzip/deflate`；未知或多层 encoding 默认拒绝。
- 解压流同时限制 compressed 2 MiB、decoded 10 MiB、decoded/compressed 100:1，并受总 deadline 约束。达到限制立即停止读取并丢弃连接。
- decoded bytes 只分配到硬上限；解析器不读取无界 `String`。正文 hash 对 decoded 原始 bytes 计算。
- 10 MiB 是 Feed 文档上限，不是单条正文承诺；解析后再限制条目数 5,000、单个 title 64 KiB、单个 content 1 MiB、全部规范化 entry 内容 16 MiB。

### 6.4 状态与 MIME

- 200：受限读取、sniff、解析。
- 304：不读 body，更新成功时间、validator/cache 元数据，不改 entries。
- 204：invalid feed response。
- 3xx：只走手工 redirect。
- 401/403：auth-required/permanent，长间隔重试并提示用户。
- 404/410：not-found/gone；410 可进入人工恢复状态，不能删除历史 entry。
- 408/425/429/5xx：transient；解析合法 `Retry-After`，仍受最大 4 小时 cooldown 约束。
- 其他 4xx：permanent client response；人工刷新仍重新做安全检查。

MIME 容错：

- XML MIME、JSON Feed MIME 直接按对应 parser 尝试。
- `text/plain`、`text/html`、`application/octet-stream` 在去 BOM/前导空白后以 `<rss`、`<feed`、`<?xml`、RDF 或 JSON object 特征 sniff；sniff 不命中则拒绝。
- 明显 image/audio/video/PDF、NUL 密集或不可识别 binary 直接拒绝，错误不回显 body。

## 7. RSS/Atom 解析与 HTML 清洗

### 7.1 XML/Feed parser

- 用 HTTP Content-Type hint、BOM、XML declaration 的既定优先级检测 encoding 并严格转换为 UTF-8；XML preflight 使用 direct `quick-xml 0.41` 完整消费到 EOF，拒绝 DTD/ENTITY、深度 >128、事件/属性异常膨胀。parser bytes 只删除 leading BOM/XML declaration，禁止 event 重序列化、属性重排、空白折叠、entity 解码或 CDATA 改写，避免 feedparser-rs 二次按旧 charset 解码。JSON Feed 不走 XML preflight，但共享 decoded/body/field budgets。
- preflight 通过后用 `feedparser_rs::parse_with_limits` 解析 RSS 0.90/0.91/0.92/1.0/2.0、Atom 与 JSON Feed。调用参数固定为 10 MiB body、5,000 entries、128 depth、1 MiB parser text、64 enclosures、64 KiB attribute；随后再应用 title 64 KiB、总 normalized text 16 MiB、enclosure JSON 256 KiB 的领域上限。解析由固定 2 permit 的 process-wide semaphore 限制；`OwnedSemaphorePermit` 必须 move 进 `spawn_blocking` closure，调用方取消不能提前释放容量。
- `FeedVersion::Unknown`、任意 `bozo=true` 或非空 `bozo_exception` 全部 typed reject，不接受部分结果或字符串告警 allowlist。crate 类型立即映射为 owned domain DTO；不得让 parser 的 URL/string wrapper 或原始 HTML 跨越模块边界。
- 解析 relative link 时使用 `xml:base`，否则使用最终响应 URL；结果再次只允许 http/https 并移除 credentials/fragment。
- 日期优先 published，再 updated；都缺失时 `published_at = None`，排序使用 `inserted_at`，不要把 fetch time伪装成 publisher date。
- 单响应内重复 identity 先按稳定规则折叠：优先 updated/published 较新，其次保持文档顺序，并产生诊断计数。

### 7.2 sanitizer policy

建议允许：`p/br/hr/blockquote/pre/code/strong/em/b/i/u/s/ul/ol/li/h1..h6/table/thead/tbody/tr/th/td/a/img` 的必要子集；禁止 `script/style/form/input/button/iframe/object/embed/svg/math/video/audio`。

- 删除所有 `style`、`class`、`id`、`on*`、publisher `data-*`、`srcset`、`ping`、`formaction`。
- `a[href]` 只留 http/https，移除 publisher `target/rel`；渲染层统一安全属性。
- `img` 只保留清洗后的 https/http original URL、alt 与合理 width/height 元数据，但默认输出为无 `src` 的 inert placeholder。拒绝 SVG data URL；小型 raster data URL 如需支持需另设严格字节上限。
- 不服务端抓取 entry 中的图片、favicon、enclosure 或链接预览。enclosure 只作为 metadata 保存，用户点击后由浏览器处理。
- 清洗后再计算 entry `content_hash`，这样无意义的 publisher tracking 属性变化不会造成正文更新；另保留 raw decoded Feed hash 用于整份未变化判断。

### 7.3 XSS/隐私测试

- script、事件属性、CSS `url()`、`javascript:`/`data:text/html`、SVG、iframe、form、malformed nesting、UTF-7/编码混淆。
- anchor 安全属性与外站 URL；相对链接正确 resolve。
- remote image 初始 HTML 不产生网络请求；CSP 仍为 `img-src 'self' data:`。
- IT之家形态 fixture 应包含 style/class/data-vmark、`target=_blank`、带 query 的 HTTPS 图片，验证清洗后正文仍可读且跟踪属性消失。

## 8. 条目身份、upsert 与二次刷新幂等

### 8.1 identity 优先级

1. 非空且长度受限的 GUID/Atom id。若可解析为 URL，使用 entry URL normalization；否则作为 trimmed opaque id，保留大小写。
2. 规范化 canonical entry URL。
3. 稳定指纹：domain-separated BLAKE3(`title_norm`, `author_norm`, `published_or_updated`, `stable_text_norm`)。不得包含 fetch time。

保存 `identity_kind` 和完整 canonical identity；`identity_hash` 只用于索引，冲突时比较完整值。Feed URL 同样是 hash + 完整值双比较。

### 8.2 事务行为

- 网络、decode、parse、sanitize 完成后开启短事务。
- 以 `(feed_id, identity_hash)` upsert；hash 相同但完整 identity 不同视为碰撞并失败告警。
- 新 identity 插入；已有 identity 且 sanitized content hash 改变时更新 title/content/summary/updated_at；未改变时不重写大字段。
- `inserted_at` 永不因刷新改变；用户 `entry_states` 不受正文更新影响。
- 更新 feed validators、raw content hash、last_fetched/next_fetch/error 状态与 entry upsert 在同一事务。
- 并发刷新依赖唯一约束仲裁；两个 worker 都解析成功时最多一份 entry。事务重试不能重发网络请求。

### 8.3 必测断言

- 同一 60-item fixture 第一次：60 new；第二次：0 new、0 duplicated、总数 60、entry IDs 与 inserted_at 不变。
- 第二次只有 HTML tracking 属性或空白变化：0 content updates。
- 同 identity 正文真实变化：0 new、1 updated、状态不丢。
- 同响应重复 GUID、GUID 大小写、URL fragment/default port、无 GUID fallback。
- 两个并发 refresh：总数仍为 unique identity 数，统计可有一个 winner/一个 conflict retry，但无 500 和无重复。

## 9. 错误分类与退避

内部使用明确枚举，API 只返回稳定、脱敏错误。建议类别：

- `UrlRejected`、`AddressDenied`、`RedirectRejected`：安全/permanent；不自动高频重试。
- `DnsTimeout/DnsFailure`、`ConnectTimeout`、`TlsFailure`、`FirstByteTimeout`、`ReadTimeout`：网络 transient；TLS 证书长期失败可降为 daily/manual 状态。
- `HttpRateLimited/HttpServerError`：transient，尊重 Retry-After。
- `BodyTooLarge/DecompressionLimit/UnsupportedEncoding`：resource/permanent-until-source-fixes。
- `UnsupportedContentType/ParseFailure/UnsafeXml`：content error；低频重试，允许 publisher 修复。
- `PersistConflict/PersistFailure`：storage transient；原解析结果可在同次作有界事务重试。

默认调度：基础 5 分钟，`5m * 2^error_count` 加 full jitter，最大 4 小时；合法 Retry-After 取较大值但仍 cap 4 小时。200 与 304 都重置 error_count。用户手工刷新可绕过普通 cooldown 一次，但不能绕过 SSRF、scheme、大小或并发租约限制；API 需要 rate limit 并返回 queued/running/cooldown。

日志/metrics 记录 `feed_id, host, hop_count, resolved_address_count, status, compressed_bytes, decoded_bytes, entry_count, new_count, updated_count, error_class, duration`。不记录 response body、完整 URL/query、validator 值或清洗前 HTML。

## 10. 确定性 fixture 与本地 mock server

### 10.1 fixture 集合

不要把实时 IT之家全文快照当普通 CI fixture，避免内容漂移与不必要的版权内容入库。提交手工合成 fixture：

- `rss_2_60_items.xml`：恰好 60 个 item、唯一 GUID、relative/absolute link、HTML、图片与长正文；固定 timestamps 和 expected BLAKE3 manifest。
- `atom_mixed.xml`：Atom id、xml:base、updated/published、XHTML/content。
- `rss_identity_edges.xml`：缺 GUID、重复 GUID、GUID URL normalization、无日期。
- `malicious_html.xml`：script/onerror/style/svg/iframe/form/javascript URL/malformed nesting。
- `unsafe_xml.xml`：DOCTYPE、external entity、billion-laughs 形态、超深 nesting。
- `wrong_content_type.xml`：合法 Feed 配 `text/html`，验证 sniff；另有 binary 假 Feed。
- 压缩 fixture 不必提交多份正文；测试中用固定算法参数生成 gzip/deflate/br，另保留一个预生成截断/损坏流。

每个 fixture 配小型 manifest：format、expected entries、expected identities、expected sanitized snippets、expected removed attributes、decoded hash。fixture 中时间和 UUID 全固定。

### 10.2 mock server

优先用现有 Axum/Tokio 写仓库内 `ScriptedFeedServer`，避免首版增加 dev dependency。它需要：

- 记录 method/path/headers/request count；按脚本依次返回 200 -> 304、ETag/Last-Modified、relative redirect、429/Retry-After。
- 支持 fixed、chunked、delayed headers、delayed chunks、never-ending body、gzip/br、错误 Content-Length、损坏压缩流。
- 普通场景用 Axum；需要精确慢速或畸形 HTTP framing 的场景用最小 `tokio::net::TcpListener` helper。
- loopback 放行只存在于 `src/feeds` 的 `#[cfg(test)]` policy constructor，不能成为 public runtime setting。API 集成测试使用 fake `FeedFetcher`，transport 测试放在模块内使用 test policy。
- DNS/rebinding 测试使用 fake resolver + recording connector，不依赖公网 DNS 或 `/etc/hosts`。

### 10.3 测试层级

1. 纯单元：URL/IP 表、identity、backoff、MIME sniff、validator 绑定。
2. parser/sanitizer corpus：fixtures，无网络、逐字断言重要输出。
3. fetch integration：scripted server，覆盖 redirect、304、timeout、压缩和 header。
4. repository contract：SQLite/PostgreSQL/MySQL 共用 unique/upsert/concurrent refresh 测试。
5. API integration：认证、CSRF、用户隔离、queued/cooldown、统一脱敏错误。
6. live smoke：仅手工/定时、真实 IT之家。

建议再加入 `cargo-fuzz` 的 URL normalization、MIME sniff、quick-xml preflight、feedparser-rs limit/bozo mapping、ammonia sanitize targets；固定 seed corpus 来自上述 fixture，fuzz 不进入每次普通 CI。

## 11. IT之家 opt-in live smoke

### 11.1 2026-07-16 实测基线

对 `https://www.ithome.com/rss/` 的只读请求观察到：

- 直接 HTTPS 200，无 redirect；DNS 同时有公网 IPv4 `58.42.14.35` 与 IPv6 `240e:938:a03:500::3a2a:e23`。
- `Content-Type: text/xml; charset=utf-8`，RSS 2.0，60 个 item、60 个唯一 GUID。
- Brotli 传输约 66,864 bytes，解压后约 207,934 bytes，压缩比约 3.1:1。
- 有 `Last-Modified`，未观察到 ETag；原样发送 `If-Modified-Since` 得到 304。
- 60 篇 description 中观察到约 143 个外链图片、24 个外链、184 个 style 属性、1,108 个 publisher data 属性和 208 个 class 属性；未观察到 script/event handler。真实 Feed 即使当前不恶意，也足以验证清洗不会破坏正文并移除 tracking/presentation 属性。

这些值只作为 smoke 诊断基线，不写成永久严格断言；唯一稳定硬断言是“可成功解析、条目规模约 60、无重复、清洗安全”。

### 11.2 运行方式

- 测试函数标记 `#[ignore]`，且内部再次要求 `RAINDROP_LIVE_RSS_SMOKE=1`；命令示例：`RAINDROP_LIVE_RSS_SMOKE=1 cargo test --test live_rss_ithome -- --ignored --nocapture`。
- 仅对用户明确给定的该 URL 发请求，固定 User-Agent；测试串行执行，最多两次刷新，不抓取文章页或图片。
- 使用临时 SQLite、临时用户和真实 domain/API flow；测试完成删除临时库。
- 失败时只输出 status/error class/count/timing，不打印整份 Feed 或正文。

### 11.3 验收断言

1. URL normalization 结果仍为 `https://www.ithome.com/rss/`；解析到的每个地址均为 allowed global unicast，连接 peer 在 pinned set 内。
2. 第一次刷新为 200 或在已有 validator 情况下为 304；若 200，decoded body <10 MiB、解析格式 RSS 2.0、entry count 在 50..=100，当前期望约 60。
3. 所有 identity 唯一；至少 50 条成功入库，title/link/pubDate/content 可用。
4. sanitized content 非空；不存在 script/style/on*/iframe/form/SVG 和 publisher class/data/style 属性；链接 scheme 安全；远程图片处于 inert/default-not-loaded 状态。
5. 通过条目列表 API 取得至少一条已入库记录，再通过 detail API 取得同一 entry 的完整 sanitized content；响应不包含 raw Feed XML。
6. 立即第二次刷新：允许 304，也允许 Feed 在测试窗口出现真实新文章；必须满足 `entries_after == unique(identity from both successful representations)`、数据库无 duplicate identity、已有 entry ID/inserted_at 不变。若第二次 304，则严格断言 `new_entries = 0`、总数不变。
7. 第二次请求在 validator URL 未变时带 `If-Modified-Since`；如果未来出现 ETag，也同时验证 `If-None-Match`。

Live smoke 不应因公网/CDN/DNS 短暂故障阻塞 PR；建议夜间 schedule 运行并上传仅含脱敏计数的结果。发布前可人工运行一次作为 RSS 纵向链路证据。

## 12. 建议 crate 与供应链取舍

以下为 2026-07-16 查询到的当前稳定版本，仅是实现建议，不在本任务修改 Cargo：

| crate | 当前版本 | 建议 | 取舍 |
| --- | ---: | --- | --- |
| `reqwest` / `rustls` | 0.13.4 / 0.23.42 | reqwest `default-features = false`，仅启用 `rustls-no-provider`/stream；direct rustls 仅启用 ring | 默认 feature 含 system proxy，必须关闭；不启用自动 gzip/br/deflate。应用在启动早期显式安装 process-global ring provider，冲突 hard fail；AWS-LC 不得进入 active graph。 |
| `feedparser-rs` | 0.5.5，MSRV 1.88 | 精确 `=0.5.5`、`default-features = false`，仅调用 `parse_with_limits` | 覆盖 RSS 0.90/0.91/0.92/1.0/2.0、Atom 0.3/1.0、JSON Feed 1.0/1.1，拥有 typed error、bozo 与 parser limits。默认 `http` 会引入 parser-owned reqwest/redirect/decompression，必须关闭。其 `ParseOptions`/sanitizer helper 不作为 Raindrop 安全边界。crates.io archive 的上游测试缺少共享 fixtures，Raindrop 必须以自己的确定性 corpus 作为升级门槛。 |
| `quick-xml` | 0.41.0 | 显式 direct dependency，与 feedparser-rs 共用同版本做 preflight | 避免依赖 transitive API 的隐式使用；双 parse 有少量 CPU 成本，但 body 已上限 10 MiB，安全收益更高，并包含 RUSTSEC-2026-0194/0195 修复。 |
| `ammonia` | 4.1.3，MSRV 1.80 | 独立 sanitizer policy | 成熟且专注，但 HTML parser 依赖图不小；白名单与 URL policy 必须由 Raindrop 明确定义，不能采用默认策略后不测试。 |
| `async-compression` | 0.4.42，MSRV 1.83 | `default-features = false`，只开 `tokio,gzip,zlib,brotli` | IT之家当前实际使用 Brotli。避免 `all`、xz、zstd 等无需求算法，控制 native/codec 供应链与二进制体积。 |
| `ipnet` | 2.12.0 | 可选，推荐用于固定 CIDR 表 | 小而稳定，减少手写 mask 错误；它不提供“是否公网”的权威动态判断，IANA policy 仍由项目表和测试负责。 |
| `hickory-resolver` | 0.26.1，MSRV 1.88 | 首版不直接增加；先用 OS resolver + pinned connector abstraction | 直接引入会扩大 DNS/平台依赖图；DNSSEC 也不解决 SSRF。若后续需要 TTL、DNS transport 或更细观测，再单独评审。 |
| `wiremock` | 0.6.5，edition 2024 | 首版不必增加 | 便于 header/序列响应断言，但现有 Axum/Tokio 足以构造 mock，且自定义 TCP helper 更适合慢流/畸形 framing。测试复杂度明显上升时再作为 dev-only 依赖评审。 |

供应链门槛：

- 所有新依赖先查看 owner、repository、最近 release、MSRV、license、build.rs/native code、默认 features 和 lockfile diff；不用宽泛 `features = ["all"]`。
- 运行 `cargo tree -e features` 确认没有 cookies、system proxy、native-tls、HTTP3、无用压缩算法或双 TLS stack。
- 合入前运行 RustSec/cargo-audit，并按可达性评估；已知 high/critical runtime 路径必须修复。锁文件必须提交，CI 使用 `--locked`。
- 当前完整图仅精确忽略既有 `RUSTSEC-2023-0071`（sqlx-mysql 只调用 `RsaPublicKey::encrypt`，不含被攻击的私钥解密）与 `RUSTSEC-2026-0173`（build-time SeaORM proc macro）；每次 SeaORM/sqlx 升级重新审查 inverse path，feedparser-rs 子树不得借用该例外。
- feedparser-rs/ammonia/quick-xml 的升级必须运行安全 corpus、bozo/limit mapping、sanitizer golden tests 和 live smoke；不能只依据 semver 自动更新。

## 13. 建议实现/验证拆分

1. `feeds/url.rs`：normalization、scheme policy、CIDR table、fake resolver tests。
2. `feeds/fetch.rs`：manual redirect、pinned connector、timeouts、bounded raw/decode stream、conditional request tests。
3. `feeds/parse.rs`：MIME sniff、encoding conversion、quick-xml preflight、feedparser-rs limit/bozo mapping、RSS/Atom/RDF/JSON fixtures。
4. `content/sanitize.rs`：Raindrop allowlist、anchor/image policy、golden/XSS tests。
5. migration/repository：Feed/entry schema、identity hash + full comparison、upsert/concurrency contract。
6. domain/API：authenticated subscribe/refresh、queued/cooldown、list/detail、uniform error mapping。
7. deterministic 60-item end-to-end：mock fetch -> parse -> clean -> SQLite -> list/detail -> second refresh idempotency。
8. ignored IT之家 live smoke；最后运行 fmt、clippy、all tests、audit、`git diff --check`。

RSS core 只有在 deterministic 60-item 测试、SSRF/rebinding/redirect/size/XSS 测试、三数据库幂等契约和 opt-in live smoke 全部有证据时，才应勾选 `tasks/todo.md` 对应项。
