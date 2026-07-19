# Raindrop

Raindrop 是使用 Rust、Axum、SeaORM 和 React 构建的自托管 RSS 阅读器。它提供安全的 Feed 抓取与正文清洗、分类管理、未读与收藏状态、Feed 内搜索、批量已读、键盘导航、响应式阅读界面，以及 SQLite、PostgreSQL、MySQL 三种数据库支持。生产 Web 界面会嵌入单个 Rust 可执行文件。

## v0.2.0

- 支持 OPML 预览、原子导入和导出，可保留订阅分类与自定义标题，并自动识别重复或无效订阅。
- 阅读器来源树、文章队列、文章工具栏和移动端详情体验进一步对齐 CommaFeed 的高频阅读流程。
- 设置界面拆分为外观与订阅分页，在桌面、短屏和移动端均保持操作可达。
- 本地管理员密码只要求非空，不再限制至少 12 个字符。

## 界面预览

桌面端同时呈现来源、文章队列和正文；移动端收敛为专注阅读视图。以下截图来自本地 `v0.2.0` 实例实际订阅并刷新 `https://www.ithome.com/rss/` 后的界面。

![Raindrop 桌面阅读器：来源、文章队列与正文三栏视图](docs/assets/screenshots/reader-desktop.png)

<p align="center">
  <img src="docs/assets/screenshots/reader-mobile.png" width="390" alt="Raindrop 移动端文章阅读视图">
</p>

当前 binary 会内嵌、验签并编译官方 `raindrop.ai-content@1.0.0` Wasm Component。完成设置后，进程会把该 installation 幂等同步到数据库；配置 `RAINDROP_PROVIDER_SECRET_KEYS` 时启动真实的 provider/Wasm 内容 worker，未配置时保持惰性，不生成临时密钥也不影响 RSS。provider 管理 API/UI、内容入队入口、Reader 摘要/翻译 sidecar、Feed 生命周期投递和 MCP transport 仍是后续工作，因此当前界面不会主动发起模型请求。

## 环境要求

- Rust 1.94.0，仓库中的 `rust-toolchain.toml` 会选择该版本。
- `wasm32-unknown-unknown` target；仓库工具链文件、CI 和 Docker builder 会自动安装。
- Node.js 26.4.0 和 npm 12.0.1。

仓库提交了 `Cargo.lock` 与 `web/package-lock.json`。安装前端依赖时禁用依赖脚本，并使用 `npm ci` 保持锁文件不变。

## 交互式 SQLite 首次启动

先构建生产 Web 资源和 release binary：

```bash
npm --prefix web ci --ignore-scripts
npm --prefix web run build
cargo build --release --locked
mkdir -p data
chmod 700 data
./target/release/raindrop
```

终端会输出一次性 setup token。打开 `http://127.0.0.1:8080`，输入该 token，保留向导默认的 `sqlite://data/raindrop.db?mode=rwc`，再创建管理员。向导完成后会写入 `data/config.toml`，同一进程随即进入登录状态。

setup token 只应出现在受控终端中。不要把它写入命令行参数、截图、日志收集或工单。若在完成设置前重启，旧 token 会失效，新进程会生成新的 token。

## 环境托管初始化

`.env.example` 包含安全的本地默认值和明确的秘密占位符。Raindrop 不会自动读取 `.env`，可以由 shell、容器平台或服务管理器注入变量。

```bash
cp .env.example .env
${EDITOR:-vi} .env
mkdir -p data
chmod 700 data
set -a
. ./.env
set +a
./target/release/raindrop
```

启动前必须替换 `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` 占位符。数据库没有用户时，完整的 `RAINDROP_BOOTSTRAP_ADMIN_USERNAME` 和 `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` 会创建首位管理员。创建成功后，应从部署环境中删除整组 `RAINDROP_BOOTSTRAP_ADMIN_*` 变量；只保留用户名会被视为不完整配置。已有用户不会被完整的 bootstrap 变量覆盖。

变量、TOML 字段、优先级、反向代理和数据库说明见 [配置文档](docs/configuration.md)。

## Docker

镜像内的进程以 `10001:10001` 运行，监听 `0.0.0.0:8080`，数据目录固定为 `/data`。运行镜像不包含 Node.js、npm、Cargo、rustc 或源码。

本地构建镜像：

```bash
docker build --tag raindrop:dev .
```

本地普通 binary 和镜像使用公开、确定性的 development 签名根，并在启动内容 runtime 时明确记录该模式，只适合开发和 CI smoke。正式 tag binary 与 Docker 发布必须使用受保护的官方 seed，不能回退到 development 签名。

正式发布始终推送到 `ghcr.io/ca-x/raindrop`。以下示例使用 `latest`，生产部署也可以改用完整版本标签或 `sha-<commit>` 标签。

### 使用设置向导

不传 `RAINDROP_DATABASE_URL` 时，容器保留 Web 设置向导。命名 volume 会保存 SQLite、配置和后续数据：

```bash
docker volume create raindrop-data
docker run --detach \
  --name raindrop \
  --restart unless-stopped \
  --publish 127.0.0.1:8080:8080 \
  --volume raindrop-data:/data \
  ghcr.io/ca-x/raindrop:latest
docker logs raindrop
```

从受控终端读取一次性 setup token，再打开 `http://127.0.0.1:8080`。向导默认的 `sqlite://data/raindrop.db?mode=rwc` 在容器中解析为 `/data/raindrop.db`。能够读取 Docker 日志的人也能看到尚未使用的 setup token，应限制 daemon、日志平台和运维账号的访问。

宿主机目录挂载必须允许 UID/GID `10001:10001` 写入。普通 Docker 环境可以先准备目录：

```bash
sudo install -d -o 10001 -g 10001 -m 0700 /srv/raindrop
docker run --detach \
  --name raindrop \
  --publish 127.0.0.1:8080:8080 \
  --volume /srv/raindrop:/data \
  ghcr.io/ca-x/raindrop:latest
```

rootless Docker 或启用 user namespace remap 时，宿主机 UID 映射由 Docker 配置决定，优先使用命名 volume。

### 使用环境变量初始化

环境托管部署可以通过只允许管理员读取的 env 文件连接 SQLite、PostgreSQL 或 MySQL。容器内 SQLite 应使用 `/data` 下的绝对路径：

```dotenv
RAINDROP_PUBLIC_URL=https://rss.example.com
RAINDROP_DATABASE_URL=sqlite:///data/raindrop.db?mode=rwc
RAINDROP_BOOTSTRAP_ADMIN_USERNAME=admin
RAINDROP_BOOTSTRAP_ADMIN_PASSWORD=CHANGE_ME_WITH_A_STRONG_PASSWORD
```

也可以把数据库 URL 改为外部 PostgreSQL 或 MySQL。不要把 env 文件提交到仓库：

```bash
sudo chmod 600 /etc/raindrop/raindrop.env
docker run --detach \
  --name raindrop \
  --restart unless-stopped \
  --publish 127.0.0.1:8080:8080 \
  --env-file /etc/raindrop/raindrop.env \
  --volume raindrop-data:/data \
  ghcr.io/ca-x/raindrop:latest
```

首位管理员创建成功后，从 env 文件中删除整组 `RAINDROP_BOOTSTRAP_ADMIN_*` 变量并重建容器。反向代理终止 TLS 时，应设置浏览器实际访问的 `RAINDROP_PUBLIC_URL`，保留外部 `Host`，并只把容器端口暴露给代理。完整规则见 [Session cookie 与反向代理](docs/configuration.md#session-cookie-与反向代理)。

Docker healthcheck 和外部存活探针使用同一个无认证端点：

```bash
curl --fail --silent --show-error http://127.0.0.1:8080/api/v1/health/live
docker inspect --format '{{.State.Health.Status}}' raindrop
```

## 开发模式

后端 debug build 不嵌入 `web/dist`，根页面会提示使用 Vite。分别启动后端和前端：

```bash
npm --prefix web ci --ignore-scripts
mkdir -p data
cargo run
```

```bash
npm --prefix web run dev
```

打开 `http://127.0.0.1:5173`。Vite 会把 `/api` 请求代理到 `http://127.0.0.1:8080`，setup token 仍由后端终端输出。

## Production single binary

release build 会在编译时嵌入 `web/dist`。构建完成后，运行环境不再需要 Node.js 或单独的静态文件服务器。

```bash
npm --prefix web ci --ignore-scripts
npm --prefix web run build
cargo build --release --locked
mkdir -p data
chmod 700 data
./target/release/raindrop --version
./target/release/raindrop
```

上面的本地命令生成 development-signed 内嵌组件。需要在受控环境手工构建官方产物时，先把一份无 padding URL-safe base64 编码的 32 字节 Ed25519 seed 写入 secret manager，再通过不回显的环境注入设置：

```text
RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE=1
RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID=raindrop-release-2026
RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED=<secret-manager injection>
```

seed 缺失、格式错误或 key ID 不匹配时构建会失败且不回显值。seed 不会进入最终 binary；binary 只包含组件、规范化 manifest、SHA-256、Ed25519 signature 和对应 public key。

默认数据目录和 SQLite URL 都相对于进程工作目录。服务管理器应固定 `WorkingDirectory`，或者同时配置绝对的 `RAINDROP_DATA_DIR` 与 SQLite URL。

## 发布产物

`v*` tag 必须与 `Cargo.toml` 中的包版本一致。仓库 secret `RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED` 必须保存无 padding URL-safe base64 编码的 32 字节 seed。tag workflow 会强制官方签名并构建五个平台归档：

- Linux amd64：`x86_64-unknown-linux-gnu`
- Linux arm64：`aarch64-unknown-linux-gnu`
- Windows amd64：`x86_64-pc-windows-msvc`
- macOS amd64：`x86_64-apple-darwin`
- macOS arm64：`aarch64-apple-darwin`

每个归档包含 Raindrop 可执行文件、`README.md`、`LICENSE` 和 `.env.example`。GitHub Release 额外提供排序后的 `SHA256SUMS`。手工运行 binary workflow 只保留 GitHub Actions artifacts，不创建 GitHub Release。

Docker workflow 为 `linux/amd64` 和 `linux/arm64` 发布 GHCR 镜像，并附带 provenance 与 SBOM。官方 seed 只通过 BuildKit secret mount 进入单次 builder 进程，不作为 build argument、层、镜像环境变量或 artifact；每次发布还使用非秘密 cache epoch 避免 seed 轮换时复用旧签名层。配置 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN` 后，同一组标签也会发布到 `czyt/raindrop`。tag 发布包含原始 `v*`、完整 semver、`major.minor`、`latest` 和 `sha-<commit>` 标签。手工补发必须提供已经存在且与 `Cargo.toml` 版本一致的 `release_tag`，workflow 会从该 tag 的提交重新生成同一组标签。

## 测试

基础检查入口：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
npm --prefix web ci --ignore-scripts
npm --prefix web run astryx:check
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cargo build --release --locked
```

Playwright 端到端测试会重新构建生产资源，运行 release embedded tests，构建 release binary，再启动临时实例完成设置、登录和响应式界面检查：

```bash
npm --prefix web run test:e2e:install-browser
npm --prefix web run test:e2e
```

本机已有 Chromium 时，可以避免下载浏览器：

```bash
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm --prefix web run test:e2e
```
