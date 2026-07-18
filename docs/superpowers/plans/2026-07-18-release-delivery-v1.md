# Raindrop Release Delivery v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans task-by-task. This repository is explicitly configured for inline main-Agent execution; do not dispatch subagents. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce reproducible single-binary release artifacts and a non-root multi-architecture Docker image with automated GitHub release, GHCR, optional Docker Hub, health, and smoke verification.

**Architecture:** The committed Web bundle remains the only input embedded into the Rust release binary. A three-stage Dockerfile builds Web, builds Rust, and copies only the executable plus runtime CA/curl dependencies into a non-root Debian image. GitHub Actions separately builds five native binary targets, packages documentation and checksums, and publishes one multi-architecture image; a repository-owned Node contract verifier freezes the release surface without adding runtime dependencies.

**Tech Stack:** Rust 1.94.0 edition 2024, Node.js 26.4.0/npm 12.0.1, Debian Bookworm, Docker Buildx/QEMU, GitHub Actions, GHCR, optional Docker Hub.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-16-raindrop-design.md` section 18 exactly.
- Follow the supplied Owl Docker workflow pattern: QEMU, Buildx, metadata, GHCR, optional Docker Hub, amd64/arm64, and GHA cache.
- Pin every third-party GitHub Action to an immutable commit SHA and retain the human-readable version comment.
- Do not add a Rust or npm dependency.
- The runtime image runs as UID/GID `10001`, owns `/data`, binds `0.0.0.0:8080`, and contains no Node, npm, Cargo, rustc, source tree, or build cache.
- Container startup without `RAINDROP_DATABASE_URL` must keep the existing Web setup wizard and persist its default relative SQLite database under `/data`.
- Health uses the existing unauthenticated `GET /api/v1/health/live`; do not invent a second health protocol.
- Manual binary workflow runs upload artifacts only. Git tags `v*` additionally create the GitHub Release and checksums.
- GHCR publishing is always enabled. Docker Hub publishing is enabled only when both Docker Hub secrets are non-empty.
- Do not touch `.superpowers/research/` or root `node_modules/`.
- Each completed task is committed and pushed to `feature/foundation-bootstrap` before the next task.

---

### Task 1: Non-root production image and release contract verifier

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`
- Create: `web/scripts/verify-release-contracts.mjs`
- Modify: `web/package.json`

**Interfaces:**
- Produces `npm --prefix web run check:release-contracts` for later workflows and CI.
- Produces a local image contract with `/usr/local/bin/raindrop`, `/data`, port `8080`, and `/api/v1/health/live`.

- [x] **Step 1: Write the RED release verifier**

Create `web/scripts/verify-release-contracts.mjs` using only `node:fs`, `node:path`, and `node:url`. It resolves the repository root from the script directory, reads repository files with a bounded helper, and fails with `release contract violation: <message>`. Initial checks require:

```js
const dockerfile = read("Dockerfile")
requireMatch(dockerfile, /^FROM node:26\.4\.0-bookworm-slim AS web-builder$/mu, "pinned Node builder")
requireMatch(dockerfile, /^FROM rust:1\.94\.0-bookworm AS rust-builder$/mu, "pinned Rust builder")
requireMatch(dockerfile, /^FROM debian:bookworm-slim AS runtime$/mu, "minimal runtime stage")
requireMatch(dockerfile, /^USER 10001:10001$/mu, "non-root runtime user")
requireMatch(dockerfile, /\/api\/v1\/health\/live/u, "existing liveness endpoint")
requireMatch(dockerfile, /^VOLUME \["\/data"\]$/mu, "persistent data volume")
requireMatch(dockerfile, /^ENTRYPOINT \["\/usr\/local\/bin\/raindrop"\]$/mu, "exec-form entrypoint")
```

Add the root script:

```json
"check:release-contracts": "node scripts/verify-release-contracts.mjs"
```

- [x] **Step 2: Run and confirm RED**

Run: `npm --prefix web run check:release-contracts`

Expected: non-zero with `release contract violation: required file is missing: Dockerfile`.

- [x] **Step 3: Implement the Dockerfile and context boundary**

Use `# syntax=docker/dockerfile:1.7`. The Web stage runs locked `npm ci --ignore-scripts` and `npm run build`. The Rust stage copies the repository after `.dockerignore`, replaces `web/dist` from the Web stage, and runs:

```dockerfile
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked && \
    cp target/release/raindrop /tmp/raindrop
```

The final stage installs only `ca-certificates` and `curl`, creates UID/GID `10001`, copies the binary, declares OCI build metadata args/labels, then sets:

```dockerfile
ENV RAINDROP_DATA_DIR=/data \
    RAINDROP_BIND=0.0.0.0:8080
WORKDIR /
VOLUME ["/data"]
EXPOSE 8080
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/api/v1/health/live"]
ENTRYPOINT ["/usr/local/bin/raindrop"]
```

`.dockerignore` excludes `.git`, `.github`, `.superpowers`, `target`, both `node_modules` trees, `web/dist`, test results, local data/config/database files, and editor/OS noise while retaining source, lockfiles, README, LICENSE, and `.env.example`.

- [x] **Step 4: Verify the image contract**

Run:

```bash
npm --prefix web run check:release-contracts
npm --prefix web run build
cargo build --release --locked
git diff --check
```

Expected: all pass. Docker CLI absence is recorded; actual build/run is a blocking Task 3 CI gate.

- [x] **Step 5: Commit and push**

```bash
git add Dockerfile .dockerignore web/scripts/verify-release-contracts.mjs web/package.json docs/superpowers/plans/2026-07-18-release-delivery-v1.md
git commit -m "build: add production container"
git push origin feature/foundation-bootstrap
```

### Task 2: Cross-platform binary artifacts and checksums

**Files:**
- Create: `.github/actionlint.yaml`
- Create: `.github/workflows/release-binaries.yml`
- Modify: `web/scripts/verify-release-contracts.mjs`

**Interfaces:**
- Consumes `web/dist` and the release binary embedding contract.
- Produces five archives plus `SHA256SUMS` on `v*` tags; manual runs retain downloadable workflow artifacts without creating a release.

- [x] **Step 1: Extend the verifier RED contract**

Require `release-binaries.yml` to contain `push.tags: v*`, `workflow_dispatch`, read-only default permissions, five frozen targets, Web artifact upload/download, README/LICENSE/`.env.example`, immutable action SHAs, SHA-256 generation, and a tag-only release job.

- [x] **Step 2: Run and confirm RED**

Run: `npm --prefix web run check:release-contracts`

Expected: non-zero with `required file is missing: .github/workflows/release-binaries.yml`.

- [x] **Step 3: Implement the binary workflow**

Create one `web-assets` job and a native matrix with:

```yaml
include:
  - os: ubuntu-24.04
    target: x86_64-unknown-linux-gnu
    extension: ""
    archive: tar.gz
  - os: ubuntu-24.04-arm
    target: aarch64-unknown-linux-gnu
    extension: ""
    archive: tar.gz
  - os: windows-latest
    target: x86_64-pc-windows-msvc
    extension: .exe
    archive: zip
  - os: macos-15-intel
    target: x86_64-apple-darwin
    extension: ""
    archive: tar.gz
  - os: macos-15
    target: aarch64-apple-darwin
    extension: ""
    archive: tar.gz
```

Each job installs Rust 1.94.0, downloads `web/dist`, builds `--release --locked --target`, verifies `v<package version>` on tags, packages the binary with README, LICENSE, and `.env.example`, and uploads the archive. The release job runs only for tag refs, downloads all archives with `merge-multiple: true`, creates sorted `SHA256SUMS`, and publishes through pinned `softprops/action-gh-release` with `generate_release_notes: true`.

- [x] **Step 4: Verify workflow contracts and syntax**

Run:

```bash
npm --prefix web run check:release-contracts
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.7 .github/workflows/release-binaries.yml
git diff --check
```

Expected: all pass.

- [x] **Step 5: Commit and push**

```bash
git add .github/actionlint.yaml .github/workflows/release-binaries.yml web/scripts/verify-release-contracts.mjs docs/superpowers/plans/2026-07-18-release-delivery-v1.md
git commit -m "ci: package release binaries"
git push origin feature/foundation-bootstrap
```

### Task 3: Multi-architecture image publishing and blocking container smoke

**Files:**
- Create: `.github/workflows/docker.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `web/scripts/verify-release-contracts.mjs`

**Interfaces:**
- Produces `ghcr.io/czyt/raindrop` and optional `czyt/raindrop` tags for linux/amd64 and linux/arm64.
- Produces a blocking PR/push container build-and-health smoke in `ci.yml`.

- [ ] **Step 1: Extend the verifier RED contract**

Require Docker publish triggers, `packages: write`, QEMU, Buildx, unconditional GHCR login, conditional Docker Hub login, dynamic public image list, semver/tag/latest/sha metadata, `linux/amd64,linux/arm64`, GHA cache, provenance/SBOM, and immutable SHAs. Require `ci.yml` to run the release verifier and define `container-smoke` with a single-platform Buildx load, non-root inspection, live health polling, and cleanup trap.

- [ ] **Step 2: Run and confirm RED**

Run: `npm --prefix web run check:release-contracts`

Expected: non-zero with `required file is missing: .github/workflows/docker.yml`.

- [ ] **Step 3: Implement Docker publishing**

The workflow uses the reviewed action pins and computes the image list without exposing secrets:

```yaml
- name: Select public image names
  id: images
  env:
    DOCKERHUB_USERNAME: ${{ secrets.DOCKERHUB_USERNAME }}
    DOCKERHUB_TOKEN: ${{ secrets.DOCKERHUB_TOKEN }}
  run: |
    printf 'images<<EOF\n' >> "$GITHUB_OUTPUT"
    printf 'ghcr.io/%s\n' "${GITHUB_REPOSITORY,,}" >> "$GITHUB_OUTPUT"
    if [[ -n "$DOCKERHUB_USERNAME" && -n "$DOCKERHUB_TOKEN" ]]; then
      printf 'czyt/raindrop\n' >> "$GITHUB_OUTPUT"
    fi
    printf 'EOF\n' >> "$GITHUB_OUTPUT"
```

Metadata includes tag ref, semver version, major.minor, tag-only `latest`, and `sha-<short>`. Build args are `VERSION`, UTC `BUILD_TIME`, and `GIT_COMMIT`; Buildx pushes both platforms with `cache-from/cache-to type=gha`, provenance, and SBOM.

- [ ] **Step 4: Add the blocking CI container smoke**

After Web/Rust jobs, Buildx loads `linux/amd64` as `raindrop:ci`, then the smoke step:

```bash
set -euo pipefail
container=raindrop-ci-smoke
trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT
docker run --detach --name "$container" --publish 127.0.0.1::8080 raindrop:ci
test "$(docker inspect --format '{{.Config.User}}' "$container")" = "10001:10001"
port="$(docker port "$container" 8080/tcp | sed -n 's/.*://p')"
for attempt in {1..30}; do
  curl --fail --silent "http://127.0.0.1:${port}/api/v1/health/live" && break
  test "$attempt" -lt 30
  sleep 1
done
test "$(docker inspect --format '{{.State.Health.Status}}' "$container")" = "healthy"
```

- [ ] **Step 5: Verify and observe remote CI**

Run local contracts and syntax parsing, commit/push, then use `gh run list`/`gh run watch` to require the branch CI `container-smoke` job to pass. Do not dispatch the publishing workflow or create registry packages from a feature branch.

- [ ] **Step 6: Commit and push**

```bash
git add .github/workflows/docker.yml .github/workflows/ci.yml web/scripts/verify-release-contracts.mjs docs/superpowers/plans/2026-07-18-release-delivery-v1.md
git commit -m "ci: publish multi-arch container"
git push origin feature/foundation-bootstrap
```

### Task 4: Deployment documentation, task state, and final report

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `tasks/todo.md`
- Modify: `tasks/plan.md`
- Create: `.superpowers/sdd/release-delivery-v1-report.md`

**Interfaces:**
- Produces operator instructions for setup-wizard Docker, environment-managed Docker, persistent storage, reverse proxy, health, and release verification.

- [ ] **Step 1: Document exact container workflows**

Add build/run examples, `/data` ownership, setup token handling, `RAINDROP_PUBLIC_URL`, external PostgreSQL/MySQL, health endpoint, tags, GHCR/Docker Hub behavior, and release archive contents. Do not claim OIDC, OPML, AI/plugin/MCP, or final release readiness.

- [ ] **Step 2: Run bounded final gates**

```bash
npm --prefix web run check:release-contracts
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.7
npm --prefix web run check:reader-types
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cargo fmt --check
cargo test --locked --all-features
git diff --check
```

- [ ] **Step 3: Update authoritative state**

Mark only release CI quality, binary/checksum workflow, Docker workflow, and non-root container documentation complete. Keep the SeaORM future-incompatibility item open until upstream is upgraded. Keep OIDC, OPML, AI/plugin/MCP, full portability CI, and release smoke open.

- [ ] **Step 4: Commit and push**

```bash
git add README.md docs/configuration.md tasks/todo.md tasks/plan.md docs/superpowers/plans/2026-07-18-release-delivery-v1.md
git add -f .superpowers/sdd/release-delivery-v1-report.md
git commit -m "docs: document release delivery"
git push origin feature/foundation-bootstrap
```

## Plan self-review

- Spec coverage: binary targets, embedded Web assets, documentation archives, checksums, tag release, multi-architecture GHCR, optional Docker Hub, non-root runtime, `/data`, health, GHA cache, and operator docs each map to a task.
- Cloud-native consistency: runtime configuration remains environment-driven, the process receives SIGTERM directly as PID 1, health reuses the existing endpoint, and persistent local state is isolated to `/data`; multi-node deployments are explicitly directed to PostgreSQL/MySQL.
- Security consistency: actions are SHA-pinned, workflow permissions are least-privilege, Docker Hub secrets only control login/image selection, build tools do not enter runtime, and setup/database secrets are never printed by smoke tests.
- Type/name consistency: image names are exactly `ghcr.io/${github.repository}` and `czyt/raindrop`; container user is exactly `10001:10001`; port is exactly `8080`; liveness path is exactly `/api/v1/health/live`.
- Placeholder scan: no deferred code, undefined interface, or generic verification instruction remains.
- Scope exclusions: OIDC, OPML, AI/plugin/MCP, full release smoke, and unresolved dependency advisories remain explicit backlog work.
