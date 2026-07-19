# Raindrop 配置

本文记录当前已实现的运行时配置合同。未列出的运行时 `RAINDROP_*` 变量不会改变程序行为。AI provider 独立加密 keyring、官方内嵌插件同步和生产内容 worker 已接入启动生命周期；provider/content 管理 API/UI、自动入队、Reader sidecar、生命周期 dispatcher 和 MCP transport 尚未接通，OIDC 也仍未实现。

## 加载顺序

配置按字段合并，优先级从高到低为：

1. 已识别的 `RAINDROP_*` 环境变量。
2. 数据目录中的 `config.toml`。
3. 程序默认值。

`RAINDROP_DATA_DIR` 会先决定数据目录和配置文件位置。未设置时，数据目录为当前工作目录下的 `data`，配置文件为 `data/config.toml`。当前 binary 没有 `--config` 参数。

环境变量按字段覆盖 TOML，同一组配置可以混用两种来源。已识别的空值仍会参与解析，不能用空字符串恢复默认值。未知环境变量会被忽略；已识别但非法的值会使启动失败。

## Stage 1 环境变量

| 环境变量 | TOML 字段 | 默认值与合同 |
| --- | --- | --- |
| `RAINDROP_DATA_DIR` | 无 | `data`。决定 `config.toml` 的位置，不会自动改写数据库 URL。 |
| `RAINDROP_BIND` | `bind` | `0.0.0.0:8080`。必须是 `IP:port`，不能填写主机名。 |
| `RAINDROP_PUBLIC_URL` | `public_url` | 未设置。必须是带主机的绝对 `http://` 或 `https://` URL。当前主要用于决定 session cookie 是否带 `Secure`。 |
| `RAINDROP_DATABASE_URL` | `database_url` | 未设置。支持 `sqlite://`、`postgres://`、`postgresql://`、`mysql://`。存在时连接受管数据库；空库会自动创建管理员或进入仅管理员设置模式。 |
| `RAINDROP_FEED_ORPHAN_RETENTION_DAYS` | `feed_orphan_retention_days` | `30`。必须是 `0..=3650` 的整数天；`0` 禁用 orphan Feed 的物理清理。 |
| `RAINDROP_SESSION_SECRET` | `session_secret` | 未设置。设置后至少 32 字节，否则启动失败。当前 foundation 会加载并脱敏该值，但浏览器会话令牌仍由系统随机源独立生成，因此交互式设置不要求提供此变量。 |
| `RAINDROP_PROVIDER_SECRET_KEYS` | `provider_secret_keys` | 未设置。逗号分隔的 `key-id:base64url` 列表；环境变量替换完整 TOML 列表。第一项用于新 credential 加密，后续项只用于轮换期解密。每份 key 解码后必须为 32 字节，且不得重复 ID 或 key material。详见 [AI Provider Core 运维合同](ai-providers.md)。 |
| `RAINDROP_BOOTSTRAP_ADMIN_USERNAME` | `bootstrap_admin.username` | 未设置。与管理员密码成组使用；空数据库初始化时不能为空，最终用户名必须为 3 到 64 个非空白、非控制字符。 |
| `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` | `bootstrap_admin.password` | 未设置。与管理员用户名成组使用，不能为空；生产环境仍建议使用密码管理器生成强密码。 |
| `RAINDROP_BOOTSTRAP_ADMIN_EMAIL` | `bootstrap_admin.email` | 未设置。可选；首尾空白会被移除，空值按未提供处理。非空值仅接受 ASCII，转为小写后最多 320 字节、只能有一个 `@`、local/domain 都不能为空且分别最多 64/255 字节，并拒绝空白与控制字符。该保守的未加引号地址子集不覆盖 RFC 的 quoted local-part 或国际化地址。 |

`RUST_LOG` 由 `tracing` 读取，不属于 `RAINDROP_*` 配置。未设置时使用 `raindrop=info,tower_http=info`。

## 构建期官方插件签名

以下变量只由 Cargo build script、binary 发布 workflow 和 Docker builder 消费，不属于 `RuntimeConfig`，部署容器或 binary 时设置它们没有运行时效果：

| 构建变量 | 合同 |
| --- | --- |
| `RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE` | 正式构建固定为 `1`；缺失时允许 development 签名，其他值失败。 |
| `RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID` | 正式构建固定为 `raindrop-release-2026`，只有同时提供 seed 时才允许存在。 |
| `RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED` | 无 padding URL-safe base64，解码后正好 32 字节。只从 GitHub secret 或 Docker BuildKit secret mount 进入构建进程。 |

普通本地构建和非发布 CI 使用公开、确定性的 `raindrop-development-2026` trust root，并在 runtime 初始化时记录 development 模式。`v*` binary 与 Docker workflow 设置强制开关；seed 缺失、非规范 base64、长度错误或 key ID 不匹配都会在生成产物前失败，错误不包含 seed。

构建会把官方 no-WASI Component、规范化 manifest、SHA-256 digest、Ed25519 signature 和 public key 编入 binary。seed 会在 build process 中清零，不写入 `OUT_DIR`、最终 binary、镜像层或发布 artifact。Docker workflow 禁止把 seed 作为 build argument，并使用非秘密 cache epoch 使同一 tag 的重新签名不会命中旧 builder layer。

轮换时生成新的 32 字节 seed，通过 secret manager 原子更新 `RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED`，重新运行完整 tag 构建并验证所有平台的嵌入 bundle；旧 binary 自带其匹配 public key，不依赖运行时 key server。确认新产物发布后再撤销旧 secret 备份。不要把 seed 放入 `.env`、TOML、shell 历史、工单或日志采集。

## Docker 中的路径与变量

正式镜像设置了两个默认环境变量：

```text
RAINDROP_DATA_DIR=/data
RAINDROP_BIND=0.0.0.0:8080
```

命令行 `--env`、`--env-file` 或容器平台注入的同名变量会覆盖镜像默认值。通常不需要修改这两个值，只需把 volume 或宿主机目录挂载到 `/data`，并把容器的 `8080` 端口发布给本机反向代理。

镜像的工作目录是 `/`。设置向导中的默认 URL `sqlite://data/raindrop.db?mode=rwc` 因此指向 `/data/raindrop.db`。环境托管容器建议写成绝对路径，避免以后修改工作目录时改变数据库位置：

```text
RAINDROP_DATABASE_URL=sqlite:///data/raindrop.db?mode=rwc
```

容器以 UID/GID `10001:10001` 运行。命名 volume 会保留镜像中 `/data` 的所有权；宿主机 bind mount 需要由部署方授予映射后的容器用户读写权限。不要为了绕过权限问题把容器改为 root，也不要把 SQLite 放到容器可写层或网络文件系统。

不设置数据库 URL 时，setup token 只写入容器标准错误，可以通过 `docker logs <container>` 在受控终端读取。日志采集平台可能长期保存该 token，完成设置前应限制读取权限。设置完成或容器重启后，旧 token 失效。

使用外部 PostgreSQL 或 MySQL 时，把数据库 URL 放在 secret/env 管理中，不要写进镜像。首次自动创建管理员后删除整组 `RAINDROP_BOOTSTRAP_ADMIN_*` 变量，再重建容器；只留下用户名、密码或邮箱中的一部分会导致启动配置错误。反向代理部署仍需设置浏览器实际访问的 `RAINDROP_PUBLIC_URL` 并保留外部 `Host`。

镜像的 Docker healthcheck 请求现有存活端点：

```text
GET /api/v1/health/live
```

该端点不需要会话或 setup token。它只表示进程可响应存活请求，不替代数据库备份、业务读写探针或外部监控。

## TOML 示例

手工管理配置文件时，可以使用下面的结构：

```toml
bind = "127.0.0.1:8080"
public_url = "https://rss.example.com"
database_url = "sqlite:///var/lib/raindrop/raindrop.db?mode=rwc"
session_secret = "REPLACE_WITH_AT_LEAST_32_RANDOM_BYTES"
provider_secret_keys = ["primary:<BASE64URL_32_BYTES>"]
feed_orphan_retention_days = 30

[bootstrap_admin]
username = "admin"
password = "REPLACE_WITH_A_STRONG_PASSWORD"
email = "admin@example.com"
```

不要把示例占位符当作秘密使用。provider keyring 不得复用 session secret；轮换和备份规则见 [AI Provider Core 运维合同](ai-providers.md)。手工创建的文件不会被程序自动修正权限，在 Unix 上应执行：

```bash
chmod 600 /var/lib/raindrop/config.toml
chmod 700 /var/lib/raindrop
```

## 两种设置模式与自动初始化

### 设置向导

环境变量和 TOML 都没有提供数据库 URL 时，程序进入 `FULL` 设置模式：

1. 每次进程启动都会生成新的 setup token，并输出到标准错误。
2. 数据库检查和完成设置都必须通过 `X-Setup-Token` 提交该 token，包括 loopback 请求。
3. 向导默认填写 `sqlite://data/raindrop.db?mode=rwc`，也可以改为 PostgreSQL 或 MySQL URL。连接前会创建数据目录；Unix 上目录权限设为 `0700`。
4. 数据库连接、迁移和管理员创建成功后，当前进程会拒绝后续设置请求。

setup token 不会通过 bootstrap API 返回。服务只保留 token 的 BLAKE3 哈希用于比较。完成设置前重启会生成新 token；完成后 `database_url` 已写入配置，后续启动不会再生成 token。

设置向导先验证管理员字段，再在数据目录中创建同目录临时文件；写入失败后会尽力删除该秘密临时文件。Unix 会以 `0600` 创建文件、同步临时文件、同目录重命名为 `config.toml`，再打开并 `fsync` 父目录。Windows 会同步临时文件，并通过 `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` 执行同目录 write-through 替换。这些平台支持的 durable boundary 都发生在管理员数据库事务提交之前；替换或持久化失败不会提交管理员。

durable boundary 成功后，配置不再被删除或回滚。若管理员密码哈希或数据库事务随后失败，当前进程保持 `ADMIN_ONLY`，可以使用同一 setup token 调用 `/api/v1/setup/admin` 重试。崩溃重启时，只有“零用户且无 bootstrap claim”会进入 `ADMIN_ONLY`；用户与 singleton claim 同时存在才进入 `READY`。claim 与用户数量不一致是显式启动错误，删除已认领管理员不会重新开放 bootstrap。Windows 没有 POSIX `0600`/`0700`，需要使用平台 ACL 保护数据目录和配置文件；当前不支持的其他非 Unix/非 Windows 目标会在 durable boundary 失败关闭。

向导只新增或更新 `database_url`，已有 TOML 字段会保留。数据库 URL 以明文保存在 `config.toml`，文件权限和备份访问控制仍由管理员负责。

### 环境托管与 `ADMIN_ONLY`

只要 `RAINDROP_DATABASE_URL` 或 TOML `database_url` 存在，程序就连接数据库并运行迁移。合并后的 bootstrap 管理员用户名与密码完整时会自动创建首管理员；相关字段全部缺失时生成新的 setup token、绑定服务并进入 `ADMIN_ONLY`。部分字段存在仍是启动配置错误，不会静默回退到 Web 设置。

`ADMIN_ONLY` 的 bootstrap 响应只公开 `setupMode: "ADMIN_ONLY"`，不会返回数据库 URL 或 session 配置。浏览器复用管理员步骤，只能提交 setup token、用户名、密码和可选邮箱；数据库检查和完整设置 endpoint 都会拒绝该模式。数据库中的 singleton bootstrap claim 与用户/角色在同一事务内提交，因此多个进程竞争同一空库时只有一个首管理员成功。

Raindrop 不会自动读取 `.env`。仓库中的 [`.env.example`](../.env.example) 只用于 shell、容器平台或服务管理器的环境注入，不应提交复制后的 `.env`。

## 数据库 URL

可用格式：

```text
sqlite://data/raindrop.db?mode=rwc
sqlite:///var/lib/raindrop/raindrop.db?mode=rwc
postgres://raindrop:REPLACE_PASSWORD@db.example.internal/raindrop
postgresql://raindrop:REPLACE_PASSWORD@db.example.internal/raindrop
mysql://raindrop:REPLACE_PASSWORD@db.example.internal/raindrop
```

相对 SQLite 路径以进程工作目录为基准，不以 `RAINDROP_DATA_DIR` 为基准。完整设置会创建并保护 `RAINDROP_DATA_DIR`；使用其他相对/绝对 SQLite 路径时，其父目录仍应由部署方准备。

文件 SQLite 连接会启用外键、5 秒 busy timeout、`synchronous=NORMAL` 和 WAL，连接池上限为 1。内存 SQLite 不启用 WAL。

这些 SQLite 选项在每次连接握手时配置，而不是只对启动时取得的第一条连接执行一次 `PRAGMA`。PostgreSQL 连接同样在握手时设置 `timezone=UTC`，MySQL 连接设置 `time_zone=+00:00`，因此连接池以后新建的连接也使用 UTC 会话。

RSS operational 时间列在 PostgreSQL 使用 `TIMESTAMPTZ`，在 MySQL 使用 `DATETIME(6)`，以同时保留微秒精度并避开 MySQL `TIMESTAMP` 的 2038 范围。发布者提供的源日期不会写入这些列，而是以可空的有符号 Unix 微秒 `BIGINT` 保存；它可以表示 1970 年以前和 2038 年以后的值。

不要把 SQLite 数据库、`-wal` 或 `-shm` 文件放在 NFS、SMB/CIFS、SSHFS 等网络文件系统上。WAL 依赖可靠的共享内存和文件锁；网络存储无法保证这些语义时，可能导致启动失败、锁异常或数据损坏。需要网络存储或多节点访问时，使用 PostgreSQL 或 MySQL。

### RSS 跨数据库合同测试

`tests/rss_migrations.rs` 总是使用临时文件运行 SQLite 合同。若要在本地验证 PostgreSQL 或 MySQL，可分别设置 `RAINDROP_TEST_POSTGRES_URL`、`RAINDROP_TEST_MYSQL_URL`。它们必须指向专用、可丢弃的测试数据库：测试会回滚已有 Raindrop migrations、重建 schema，并验证迁移重入行为，不应指向开发或生产数据。

```bash
RAINDROP_TEST_POSTGRES_URL='postgres://USER:PASSWORD@127.0.0.1/raindrop_test' \
  cargo test --locked --test rss_migrations postgres -- --nocapture --test-threads=1

RAINDROP_TEST_MYSQL_URL='mysql://USER:PASSWORD@127.0.0.1/raindrop_test' \
  cargo test --locked --test rss_migrations mysql -- --nocapture --test-threads=1
```

未设置相应变量时，该 backend 合同会明确标记为跳过；测试输出不会回显数据库 URL。CI 使用一次性的 PostgreSQL/MySQL service 数据库，并串行运行三个 backend 合同。

## Feed orphan 保留策略

删除最后一个订阅时，Raindrop 先给 Feed 写入 `orphaned_at`，不会在用户请求内物理删除 Feed。Feed runtime 的 scheduler lane 在数据库就绪后立即执行一次维护，此后每小时最多扫描 100 个按 `(orphaned_at, id)` 排序的候选。

只有超过宽限期、仍无任何订阅且没有 `QUEUED`/`RUNNING` 刷新任务的 orphan Feed 才会删除。每个候选都在短事务中锁定 Feed 后重新检查，多个实例可以并发执行；清理与重新订阅竞态时，订阅命令会重新发现或重建 Feed，不会把竞态暴露为内部错误。

物理删除依赖现有外键级联清除该 Feed 的 Entries、EntryStates 和刷新历史。`lifecycle_outbox` 不带 refresh-run 外键，会继续保留已提交的版本化 payload，供后续插件投递和审计使用。本策略不会执行 SQLite `VACUUM`，也不清理已订阅 Feed 的旧文章、独立刷新历史或 outbox；这些属于单独的保留与备份策略。

默认值 `30` 适合单机与普通自托管实例。需要更长的退订恢复窗口可提高到最多 `3650`；设为 `0` 会完全禁用物理清理，存储将持续增长。修改配置后需要重启进程。

## Session cookie 与反向代理

登录 cookie 名为 `raindrop_session`，属性为 `HttpOnly`、`SameSite=Lax`、`Path=/`，有效期 30 天，不设置 `Domain`。只有 `RAINDROP_PUBLIC_URL` 或 TOML `public_url` 的 scheme 为 `https` 时，cookie 才带 `Secure`。

在 TLS 终止反向代理后部署时：

- 把 `public_url` 设置为浏览器访问的外部 HTTPS URL。
- 让代理把浏览器使用的外部 `Host` 传给 Raindrop，包括非默认端口。
- 不要依赖 `X-Forwarded-Proto` 自动开启 `Secure`；当前实现不从 forwarded headers 推导 cookie 属性。
- 在配置可信代理之前，登录不从 TCP peer、`X-Forwarded-For` 或 `Forwarded` 派生硬预算。进程使用无客户端键表的 15 分钟全局安全熔断器，并限制同时进行的昂贵认证操作；账户维度只增加 5 到 100 ms 的软延迟，不会硬锁账户。
- 修改请求带有 `Origin` 时，Raindrop 会比较 `Origin` 与收到的 `Host`。代理若把 `Host` 改成内部地址，CSRF 校验会拒绝请求。

若外部站点使用 HTTPS 但 `public_url` 未设置或仍是 `http://`，浏览器会收到不带 `Secure` 的 session cookie。

## 秘密与错误输出

数据库 URL、session secret、bootstrap 管理员密码和 setup token 都按秘密值处理。配置调试输出会脱敏，已识别字段的值校验错误只报告变量名与格式，不回显原值；设置 API 也把数据库错误转换为通用响应。

这些保护不会加密环境变量或 `config.toml`，也无法清除 shell 历史、进程环境快照和外部数据库驱动日志。不要在命令行直接展开秘密，不要提交 `.env`、`config.toml` 或数据目录，也不要把带密码的数据库 URL 粘贴到日志、截图或工单。
