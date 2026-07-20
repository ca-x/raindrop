import { readFileSync, statSync } from "node:fs"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url))
const maximumContractBytes = 1024 * 1024

const dockerfile = read("Dockerfile")
const cargoManifest = read("Cargo.toml")
const readme = read("README.md")
const repositoryUrl = "https://github.com/ca-x/raindrop"
requireMatch(cargoManifest, new RegExp(escapeRegExp(repositoryUrl), "u"), "Cargo repository URL")
requireMatch(dockerfile, new RegExp(escapeRegExp(repositoryUrl), "u"), "Docker source URL")
const webBuilderStage = dockerfile.slice(
  0,
  dockerfile.indexOf("FROM rust:1.94.0-bookworm AS rust-builder"),
)
const dockerRegistryConfigIndex = webBuilderStage.indexOf(
  "ENV NPM_CONFIG_REGISTRY=https://registry.npmjs.org/",
)
const dockerLockedInstallIndex = webBuilderStage.indexOf("npm ci --ignore-scripts")
if (
  dockerRegistryConfigIndex < 0 ||
  dockerRegistryConfigIndex > dockerLockedInstallIndex
) {
  fail("Docker npm registry normalization before locked install")
}
requireMatch(
  webBuilderStage,
  /NPM_CONFIG_REPLACE_REGISTRY_HOST=always/u,
  "Docker lockfile registry host replacement",
)
requireMatch(
  dockerfile,
  /^FROM node:26\.4\.0-bookworm-slim AS web-builder$/mu,
  "pinned Node builder",
)
requireMatch(
  dockerfile,
  /^FROM rust:1\.94\.0-bookworm AS rust-builder$/mu,
  "pinned Rust builder",
)
requireMatch(
  dockerfile,
  /^FROM debian:bookworm-slim AS runtime$/mu,
  "minimal runtime stage",
)
requireMatch(
  dockerfile,
  /^ENV RUSTUP_TOOLCHAIN=1\.94\.0$/mu,
  "Docker build toolchain override",
)
requireMatch(
  dockerfile,
  /rustup target add --toolchain "\$RUSTUP_TOOLCHAIN" wasm32-unknown-unknown/u,
  "Docker official guest target",
)
requireMatch(
  dockerfile,
  /--mount=type=secret,id=raindrop_official_plugin_signing_seed/u,
  "Docker BuildKit signing secret mount",
)
requireMatch(
  dockerfile,
  /ARG RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE=0/u,
  "Docker development-signature default",
)
requireMatch(
  dockerfile,
  /RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID/u,
  "Docker official signing key ID",
)
requireMatch(
  dockerfile,
  /ARG RAINDROP_SIGNING_CACHE_EPOCH=development/u,
  "Docker signing cache epoch",
)
requireNoMatch(
  dockerfile,
  /ARG RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED/u,
  "Docker signing seed build argument",
)
requireCount(
  dockerfile,
  /npm install --global npm@12\.0\.1 --ignore-scripts/gu,
  1,
  "pinned Docker npm installation",
)
requireMatch(dockerfile, /^USER 10001:10001$/mu, "non-root runtime user")
requireMatch(
  dockerfile,
  /\/api\/v1\/health\/live/u,
  "existing liveness endpoint",
)
requireMatch(dockerfile, /^VOLUME \["\/data"\]$/mu, "persistent data volume")
requireMatch(
  dockerfile,
  /^ENTRYPOINT \["\/usr\/local\/bin\/raindrop"\]$/mu,
  "exec-form entrypoint",
)

const runtimeStage = dockerfile.slice(
  dockerfile.indexOf("FROM debian:bookworm-slim AS runtime"),
)
requireNoMatch(
  runtimeStage,
  /\b(?:node|npm|cargo|rustc)\b/u,
  "runtime image contains a build tool",
)

const binaryWorkflow = read(".github/workflows/release-binaries.yml")
for (const target of [
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
]) {
  requireMatch(binaryWorkflow, new RegExp(target, "u"), `binary target ${target}`)
}
for (const packagedFile of [
  "README.md",
  "LICENSE",
  ".env.example",
  "docs/assets/screenshots/reader-desktop.png",
  "docs/assets/screenshots/reader-mobile.png",
  "SHA256SUMS",
]) {
  requireMatch(binaryWorkflow, new RegExp(escapeRegExp(packagedFile), "u"), `packaged ${packagedFile}`)
}
for (const screenshot of [
  "docs/assets/screenshots/reader-desktop.png",
  "docs/assets/screenshots/reader-mobile.png",
]) {
  requireMatch(readme, new RegExp(escapeRegExp(screenshot), "u"), `README screenshot ${screenshot}`)
  requireAsset(screenshot)
}
requireMatch(binaryWorkflow, /^\s+tags:\s*\n\s+- ["']v\*["']$/mu, "v* binary tag trigger")
requireMatch(binaryWorkflow, /^\s+workflow_dispatch:\s*$/mu, "manual binary trigger")
requireMatch(binaryWorkflow, /^permissions:\s*\n\s+contents: read$/mu, "read-only binary permissions")
requireCount(binaryWorkflow, /^\s+name: web-dist$/gmu, 2, "isolated embedded Web artifact name")
requireNoMatch(binaryWorkflow, /^\s+name: raindrop-web-dist$/mu, "Web artifact overlaps release archive pattern")
requireMatch(binaryWorkflow, /actions\/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02/u, "pinned upload-artifact")
requireMatch(binaryWorkflow, /actions\/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093/u, "pinned download-artifact")
requireMatch(binaryWorkflow, /softprops\/action-gh-release@a06a81a03ee405af7f2048a818ed3f03bbf83c7b/u, "pinned release action")
requireMatch(binaryWorkflow, /startsWith\(github\.ref, 'refs\/tags\/v'\)/u, "tag-only GitHub release")
requireCount(
  binaryWorkflow,
  /npm install --global npm@12\.0\.1 --ignore-scripts/gu,
  1,
  "pinned binary workflow npm installation",
)
requireMatch(
  binaryWorkflow,
  /targets: \$\{\{ matrix\.target \}\},wasm32-unknown-unknown/u,
  "binary workflow official guest target",
)
requireMatch(
  binaryWorkflow,
  /RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE: ["']1["']/u,
  "binary workflow official signature requirement",
)
requireMatch(
  binaryWorkflow,
  /RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID: raindrop-release-2026/u,
  "binary workflow official signing key ID",
)
requireCount(
  binaryWorkflow,
  /RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED: \$\{\{ secrets\.RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED \}\}/gu,
  1,
  "binary workflow protected signing seed",
)
requirePinnedActions(binaryWorkflow, "release-binaries.yml")

const dockerWorkflow = read(".github/workflows/docker.yml")
requireMatch(dockerWorkflow, /^\s+packages: write$/mu, "Docker package permission")
requireMatch(dockerWorkflow, /^\s+release_tag:\s*$/mu, "manual Docker release tag input")
requireMatch(
  dockerWorkflow,
  /group: docker-\$\{\{ inputs\.release_tag \|\| github\.ref_name \}\}/u,
  "normalized Docker release concurrency",
)
requireMatch(dockerWorkflow, /^\s+timeout-minutes: 45$/mu, "bounded native image build timeout")
requireMatch(dockerWorkflow, /ref: \$\{\{ inputs\.release_tag \|\| github\.ref \}\}/u, "tag source checkout")
requireMatch(dockerWorkflow, /test "\$RELEASE_TAG" = "v\$\{package_version\}"/u, "Docker package version gate")
requireMatch(dockerWorkflow, /git rev-list -n 1 "\$RELEASE_TAG"/u, "Docker tag commit gate")
requireNoMatch(dockerWorkflow, /docker\/setup-qemu-action@/u, "Docker release must not emulate CPU architectures")
requireMatch(dockerWorkflow, /runs-on: \$\{\{ matrix\.runner \}\}/u, "native image runner matrix")
requireMatch(
  dockerWorkflow,
  /runner: ubuntu-24\.04[\s\S]{0,160}platform: linux\/amd64/u,
  "native amd64 image build",
)
requireMatch(
  dockerWorkflow,
  /runner: ubuntu-24\.04-arm[\s\S]{0,160}platform: linux\/arm64/u,
  "native arm64 image build",
)
requireMatch(dockerWorkflow, /docker\/setup-buildx-action@e468171a9de216ec08956ac3ada2f0791b6bd435/u, "pinned Buildx action")
requireMatch(dockerWorkflow, /docker\/login-action@5e57cd118135c172c3672efd75eb46360885c0ef/u, "pinned Docker login action")
requireMatch(dockerWorkflow, /docker\/metadata-action@c1e51972afc2121e065aed6d45c65596fe445f3f/u, "pinned Docker metadata action")
requireMatch(dockerWorkflow, /docker\/build-push-action@263435318d21b8e681c14492fe198d362a7d2c83/u, "pinned Docker build action")
requireMatch(dockerWorkflow, /ghcr\.io\/%s/u, "GHCR image selection")
requireMatch(dockerWorkflow, /czyt\/raindrop/u, "optional Docker Hub image")
requireMatch(dockerWorkflow, /DOCKERHUB_USERNAME/u, "Docker Hub username secret")
requireMatch(dockerWorkflow, /DOCKERHUB_TOKEN/u, "Docker Hub token secret")
requireMatch(dockerWorkflow, /platforms: \$\{\{ matrix\.platform \}\}/u, "single native image platform per build")
requireMatch(dockerWorkflow, /docker buildx imagetools create/u, "multi-platform manifest publication")
requireMatch(dockerWorkflow, /index\("amd64"\)/u, "published amd64 manifest verification")
requireMatch(dockerWorkflow, /index\("arm64"\)/u, "published arm64 manifest verification")
requireMatch(dockerWorkflow, /vnd\.docker\.reference\.type/u, "published attestation verification")
for (const tagRule of ["type=ref,event=tag", "type=semver,pattern={{version}}", "type=semver,pattern={{major}}.{{minor}}", "type=sha,prefix=sha-"]) {
  requireMatch(dockerWorkflow, new RegExp(escapeRegExp(tagRule), "u"), `Docker metadata rule ${tagRule}`)
}
requireMatch(dockerWorkflow, /type=raw,value=latest/u, "latest Docker metadata rule")
requireMatch(
  dockerWorkflow,
  /type=raw,value=latest,enable=\$\{\{ github\.event_name == 'push' \}\}/u,
  "manual rebuild cannot roll back latest",
)
requireMatch(
  dockerWorkflow,
  /type=semver,pattern=\{\{major\}\}\.\{\{minor\}\},value=\$\{\{ env\.RELEASE_TAG \}\},enable=\$\{\{ github\.event_name == 'push' \}\}/u,
  "manual rebuild cannot roll back minor alias",
)
requireMatch(dockerWorkflow, /type=raw,value=sha-\$\{\{ needs\.source\.outputs\.short_sha \}\}/u, "manual Docker source SHA tag")
requireMatch(
  dockerWorkflow,
  /printf '%s:build-%s-%s-%s\\n'/u,
  "run-unique native image handoff",
)
requireMatch(dockerWorkflow, /GIT_COMMIT=\$\{\{ env\.SOURCE_SHA \}\}/u, "Docker source revision build argument")
requireMatch(dockerWorkflow, /cache-from: type=gha,scope=raindrop-\$\{\{ matrix\.cache_scope \}\}/u, "architecture-scoped Docker GHA cache restore")
requireMatch(dockerWorkflow, /cache-to: type=gha,mode=max,scope=raindrop-\$\{\{ matrix\.cache_scope \}\}/u, "architecture-scoped Docker GHA cache save")
requireMatch(dockerWorkflow, /^\s+provenance: true$/mu, "Docker provenance")
requireMatch(dockerWorkflow, /^\s+sbom: true$/mu, "Docker SBOM")
requireMatch(
  dockerWorkflow,
  /RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE=1/u,
  "Docker official signature requirement",
)
requireMatch(
  dockerWorkflow,
  /RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID=raindrop-release-2026/u,
  "Docker official signing key ID",
)
requireMatch(
  dockerWorkflow,
  /RAINDROP_SIGNING_CACHE_EPOCH=\$\{\{ needs\.source\.outputs\.build_time \}\}/u,
  "Docker signing cache epoch",
)
requireMatch(
  dockerWorkflow,
  /raindrop_official_plugin_signing_seed=\$\{\{ secrets\.RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED \}\}/u,
  "Docker protected BuildKit signing seed",
)
requireNoMatch(
  dockerWorkflow,
  /build-args:[\s\S]{0,600}RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED/u,
  "Docker workflow signing seed build argument",
)
requirePinnedActions(dockerWorkflow, "docker.yml")

const ciWorkflow = read(".github/workflows/ci.yml")
requireMatch(ciWorkflow, /npm --prefix web run check:release-contracts/u, "release contract CI gate")
requireMatch(ciWorkflow, /^\s+container-smoke:\s*$/mu, "container smoke job")
requireMatch(ciWorkflow, /docker\/build-push-action@263435318d21b8e681c14492fe198d362a7d2c83/u, "pinned CI Docker build action")
requireMatch(ciWorkflow, /^\s+load: true$/mu, "loaded CI image")
requireMatch(ciWorkflow, /raindrop:ci/u, "CI image tag")
requireMatch(ciWorkflow, /\.Config\.User/u, "non-root container assertion")
requireMatch(ciWorkflow, /\/api\/v1\/health\/live/u, "container liveness smoke")
requireCount(
  ciWorkflow,
  /wasm32-unknown-unknown/gu,
  5,
  "CI root-compilation official guest targets",
)
requireNoMatch(
  ciWorkflow,
  /RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE/u,
  "CI smoke must remain development-signed",
)
requireCount(
  ciWorkflow,
  /npm install --global npm@12\.0\.1 --ignore-scripts/gu,
  3,
  "pinned CI npm installations",
)
requirePinnedActions(ciWorkflow, "ci.yml")

console.log("release delivery contracts are current")

function read(relativePath) {
  const path = resolve(repositoryRoot, relativePath)
  let size
  try {
    size = statSync(path).size
  } catch {
    fail(`required file is missing: ${relativePath}`)
  }
  if (size > maximumContractBytes) {
    fail(`contract file exceeds ${maximumContractBytes} bytes: ${relativePath}`)
  }
  return readFileSync(path, "utf8")
}

function requireMatch(source, pattern, message) {
  if (!pattern.test(source)) fail(message)
}

function requireNoMatch(source, pattern, message) {
  if (pattern.test(source)) fail(message)
}

function requireCount(source, pattern, expected, message) {
  const actual = [...source.matchAll(pattern)].length
  if (actual !== expected) fail(`${message}: expected ${expected}, received ${actual}`)
}

function requireAsset(relativePath) {
  let size
  try {
    size = statSync(resolve(repositoryRoot, relativePath)).size
  } catch {
    fail(`required asset is missing: ${relativePath}`)
  }
  if (size === 0 || size > maximumContractBytes) {
    fail(`required asset has invalid size: ${relativePath}`)
  }
}

function requirePinnedActions(source, file) {
  for (const match of source.matchAll(/uses:\s*[^@\s]+@([^\s#]+)/gu)) {
    if (!/^[0-9a-f]{40}$/u.test(match[1])) {
      fail(`unpinned action in ${file}: ${match[0]}`)
    }
  }
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")
}

function fail(message) {
  throw new Error(`release contract violation: ${message}`)
}
