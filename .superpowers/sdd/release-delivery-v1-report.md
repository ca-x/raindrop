# Release Delivery v1 Verification Report

Date: 2026-07-18
Branch: `feature/foundation-bootstrap`

## Delivered scope

- A three-stage production Dockerfile that builds the locked ASTRYX Web bundle, embeds it in the Rust release binary, and ships only the binary, CA certificates, and curl in the runtime image.
- A non-root runtime contract using UID/GID `10001:10001`, `/data`, `0.0.0.0:8080`, direct SIGTERM delivery, and the existing unauthenticated `/api/v1/health/live` endpoint.
- A five-target native binary workflow for Linux amd64/arm64, Windows amd64, and macOS amd64/arm64. Tagged releases add sorted `SHA256SUMS`; manual runs retain workflow artifacts without creating a GitHub Release.
- A multi-architecture Docker workflow for `linux/amd64` and `linux/arm64`, with GHCR always enabled and `czyt/raindrop` enabled only when both Docker Hub secrets exist.
- Immutable GitHub Action pins, release contract checks, dependency audit/signature gates, Buildx cache, provenance, SBOM, and a blocking container build-and-health smoke.
- Operator documentation for the setup wizard, environment-managed startup, SQLite/PostgreSQL/MySQL, `/data` ownership, setup-token log security, reverse proxy configuration, health checks, image tags, archives, and checksums.

## Delivery commits

- `ba0a15a build: add production container`
- `ebd30c6 ci: package release binaries`
- `bb71e41 ci: publish multi-arch container`
- CI portability stabilization from `c52f62a` through `a9eefa0`, covering restored release gates, portable RSS counter migrations, MySQL metadata and reserved-name handling, refresh lock invariants, fixture cleanup, query-plan assertions, and structural lock-wait detection.
- `f5feed0 build: normalize Docker npm registry`
- `44d60da test: follow reader logout menu`

## Remote CI evidence

GitHub Actions run `29636161752` at commit `44d60da748e695dbd67f3c52220d88999743e366` completed successfully:

- Supply-chain audit, ASTRYX Web, Rust 1.94 foundation, current-stable compatibility, and the Windows durable-replacement compile gate passed.
- PostgreSQL/MySQL service contracts, migration portability, refresh claims, persistence, lifecycle outbox, query, subscription, terminal atomicity, and reader-state lanes passed.
- The release gate passed 105 release-library tests, 9 embedded-Web tests, and 14 production Playwright scenarios. Lockfiles remained unchanged and `raindrop --version` returned `raindrop 0.1.0`.
- The Docker smoke built the actual image, loaded it as `linux/amd64`, verified user `10001:10001`, started the setup-wizard container, and passed both the HTTP liveness probe and Docker health status.

The feature branch did not dispatch `release-binaries.yml` or `docker.yml`, so it did not create a GitHub Release or publish registry packages.

## Deterministic verification

- `npm --prefix web run check:release-contracts`: passed.
- `go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.7`: passed for all committed workflows.
- `npm --prefix web run check:reader-types`: generated Reader contracts current.
- `npm --prefix web run typecheck`: passed.
- `npm --prefix web run test:ci`: 37 files passed, 202 tests passed.
- `npm --prefix web run build`: passed; production JavaScript is 559.92 kB before gzip and retains the existing Vite advisory above 500 kB.
- `cargo fmt --check`: passed.
- `cargo test --locked --all-features`: 425 tests passed; 1 opt-in live RSS smoke was ignored because `RAINDROP_LIVE_RSS_SMOKE=1` was not set.
- `git diff --check`: passed.
- Chinese and English punctuation gates passed for every modified Markdown file.

## Local browser evidence

- Local `agent-browser` used the release binary and a temporary SQLite database without request interception. It completed setup, observed that logout is exposed through the ASTRYX `Open menu` trigger, logged out, logged back in, and logged out again.
- `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm exec -- playwright test e2e/setup-login.spec.ts --project=desktop-production` passed the original failed scenario, 1 test in 1.7 seconds.
- Browser state, setup token, credentials, and the temporary SQLite directory were removed after verification.

## Operational contract

- Setup-wizard mode omits `RAINDROP_DATABASE_URL`, reads the one-time setup token from controlled container logs, and persists the default SQLite database plus `config.toml` under `/data`.
- Environment-managed mode supplies database and administrator bootstrap values through a protected env/secret source. After the first administrator is created, the complete `RAINDROP_BOOTSTRAP_ADMIN_*` group must be removed before recreating the container.
- Bind mounts must grant the mapped UID/GID write access. Named volumes are the default recommendation, especially with rootless Docker or user-namespace remapping.
- Reverse proxies set the browser-visible `RAINDROP_PUBLIC_URL`, preserve the external `Host`, terminate TLS, and keep port `8080` private to the proxy where possible.

## Existing advisories

- `proc-macro-error2 v2.0.1` remains a recorded future-incompatibility dependency advisory through the SeaORM dependency chain.
- Vite reports the existing 559.92 kB production JavaScript chunk above its 500 kB advisory threshold.
- Rust reports the existing `validate_counts` dead-code warning in release-only test preparation.
- GitHub reports that the pinned Docker actions still target the deprecated Node.js 20 action runtime and are currently forced onto Node.js 24 by the runner.
- The live IT Home RSS smoke remains opt-in and is not enabled unless `RAINDROP_LIVE_RSS_SMOKE=1` is set.

## Explicitly remaining

A real `v*` tag must still verify the five published archives, `SHA256SUMS`, GitHub Release creation, GHCR and optional Docker Hub manifests, provenance, and SBOM. OIDC, OPML, AI content providers, the official AI plugin and lifecycle host, MCP client/server support, full portability coverage, retention/backup work, sorting, reading cursor, registration, and administrator management remain unchecked backlog work.
