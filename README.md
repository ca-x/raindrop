# Raindrop

Raindrop 是使用 Rust、Axum、SeaORM 和 React 构建的自托管 RSS 阅读器。当前 foundation 提供 SQLite、PostgreSQL、MySQL 连接，本地管理员初始化、浏览器会话，以及嵌入单个 Rust 可执行文件的 Web 界面。

## 环境要求

- Rust 1.94.0，仓库中的 `rust-toolchain.toml` 会选择该版本。
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

启动前必须替换 `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` 占位符。数据库没有用户时，完整的 `RAINDROP_BOOTSTRAP_ADMIN_USERNAME` 和 `RAINDROP_BOOTSTRAP_ADMIN_PASSWORD` 会创建首位管理员；创建成功后，应从部署环境中删除管理员密码。已有用户不会被这些变量覆盖。

变量、TOML 字段、优先级、反向代理和数据库说明见 [配置文档](docs/configuration.md)。

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

默认数据目录和 SQLite URL 都相对于进程工作目录。服务管理器应固定 `WorkingDirectory`，或者同时配置绝对的 `RAINDROP_DATA_DIR` 与 SQLite URL。

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
