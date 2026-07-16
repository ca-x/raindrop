# Raindrop 配置

本文记录当前 foundation 已实现的配置合同。未列出的 `RAINDROP_*` 变量不会改变程序行为，OIDC、Feed、AI、插件和 MCP 配置尚未实现。

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
| `RAINDROP_SESSION_SECRET` | `session_secret` | 未设置。设置后至少 32 字节，否则启动失败。当前 foundation 会加载并脱敏该值，但浏览器会话令牌仍由系统随机源独立生成，因此交互式设置不要求提供此变量。 |
| `RAINDROP_BOOTSTRAP_ADMIN_USERNAME` | `bootstrap_admin.username` | 未设置。与管理员密码成组使用；空数据库初始化时不能为空，最终用户名必须为 3 到 64 个非空白、非控制字符。 |
| `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` | `bootstrap_admin.password` | 未设置。与管理员用户名成组使用，至少 12 字节。 |
| `RAINDROP_BOOTSTRAP_ADMIN_EMAIL` | `bootstrap_admin.email` | 未设置。可选；首尾空白会被移除，空值按未提供处理。非空值最多 320 字节、只能有一个 `@`、local/domain 都不能为空且分别最多 64/255 字节，并拒绝空白与控制字符。该保守的未加引号地址子集不覆盖 RFC 的 quoted local-part 或全部国际化形式。 |

`RUST_LOG` 由 `tracing` 读取，不属于 `RAINDROP_*` 配置。未设置时使用 `raindrop=info,tower_http=info`。

## TOML 示例

手工管理配置文件时，可以使用下面的结构：

```toml
bind = "127.0.0.1:8080"
public_url = "https://rss.example.com"
database_url = "sqlite:///var/lib/raindrop/raindrop.db?mode=rwc"
session_secret = "REPLACE_WITH_AT_LEAST_32_RANDOM_BYTES"

[bootstrap_admin]
username = "admin"
password = "REPLACE_WITH_AT_LEAST_12_RANDOM_BYTES"
email = "admin@example.com"
```

不要把示例占位符当作秘密使用。手工创建的文件不会被程序自动修正权限，在 Unix 上应执行：

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

设置向导先验证管理员字段，再在数据目录中创建同目录临时文件，写入并 `fsync` 后，在 Unix 上设置为 `0600`，再重命名为 `config.toml` 并同步目录。重命名与目录同步是 durable boundary，发生在管理员数据库事务提交之前。重命名失败只删除临时文件；目录同步失败不会提交管理员，也不会假定配置已经持久化。

durable boundary 成功后，配置不再被删除或回滚。若管理员密码哈希或数据库事务随后失败，当前进程保持 `ADMIN_ONLY`，可以使用同一 setup token 调用 `/api/v1/setup/admin` 重试。崩溃重启时，持久配置加零用户确定进入 `ADMIN_ONLY`；首管理员事务已经提交则进入 `READY`。非 Unix 平台没有 POSIX `0600`/`0700`，需要使用平台 ACL 保护数据目录和配置文件。

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

不要把 SQLite 数据库、`-wal` 或 `-shm` 文件放在 NFS、SMB/CIFS、SSHFS 等网络文件系统上。WAL 依赖可靠的共享内存和文件锁；网络存储无法保证这些语义时，可能导致启动失败、锁异常或数据损坏。需要网络存储或多节点访问时，使用 PostgreSQL 或 MySQL。

## Session cookie 与反向代理

登录 cookie 名为 `raindrop_session`，属性为 `HttpOnly`、`SameSite=Lax`、`Path=/`，有效期 30 天，不设置 `Domain`。只有 `RAINDROP_PUBLIC_URL` 或 TOML `public_url` 的 scheme 为 `https` 时，cookie 才带 `Secure`。

在 TLS 终止反向代理后部署时：

- 把 `public_url` 设置为浏览器访问的外部 HTTPS URL。
- 让代理把浏览器使用的外部 `Host` 传给 Raindrop，包括非默认端口。
- 不要依赖 `X-Forwarded-Proto` 自动开启 `Secure`；当前实现不从 forwarded headers 推导 cookie 属性。
- 登录硬限流只使用 Raindrop TCP listener 看到的真实 peer IP；在配置可信代理之前，`X-Forwarded-For` 与 `Forwarded` 不参与限流键。
- 修改请求带有 `Origin` 时，Raindrop 会比较 `Origin` 与收到的 `Host`。代理若把 `Host` 改成内部地址，CSRF 校验会拒绝请求。

若外部站点使用 HTTPS 但 `public_url` 未设置或仍是 `http://`，浏览器会收到不带 `Secure` 的 session cookie。

## 秘密与错误输出

数据库 URL、session secret、bootstrap 管理员密码和 setup token 都按秘密值处理。配置调试输出会脱敏，已识别字段的值校验错误只报告变量名与格式，不回显原值；设置 API 也把数据库错误转换为通用响应。

这些保护不会加密环境变量或 `config.toml`，也无法清除 shell 历史、进程环境快照和外部数据库驱动日志。不要在命令行直接展开秘密，不要提交 `.env`、`config.toml` 或数据目录，也不要把带密码的数据库 URL 粘贴到日志、截图或工单。
