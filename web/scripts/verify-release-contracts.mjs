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

function fail(message) {
  throw new Error(`release contract violation: ${message}`)
}
