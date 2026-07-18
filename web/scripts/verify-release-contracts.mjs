import { readFileSync, statSync } from "node:fs"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url))
const maximumContractBytes = 1024 * 1024

const dockerfile = read("Dockerfile")
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
for (const packagedFile of ["README.md", "LICENSE", ".env.example", "SHA256SUMS"]) {
  requireMatch(binaryWorkflow, new RegExp(escapeRegExp(packagedFile), "u"), `packaged ${packagedFile}`)
}
requireMatch(binaryWorkflow, /^\s+tags:\s*\n\s+- ["']v\*["']$/mu, "v* binary tag trigger")
requireMatch(binaryWorkflow, /^\s+workflow_dispatch:\s*$/mu, "manual binary trigger")
requireMatch(binaryWorkflow, /^permissions:\s*\n\s+contents: read$/mu, "read-only binary permissions")
requireMatch(binaryWorkflow, /actions\/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02/u, "pinned upload-artifact")
requireMatch(binaryWorkflow, /actions\/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093/u, "pinned download-artifact")
requireMatch(binaryWorkflow, /softprops\/action-gh-release@a06a81a03ee405af7f2048a818ed3f03bbf83c7b/u, "pinned release action")
requireMatch(binaryWorkflow, /startsWith\(github\.ref, 'refs\/tags\/v'\)/u, "tag-only GitHub release")
requirePinnedActions(binaryWorkflow, "release-binaries.yml")

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
