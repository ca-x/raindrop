import { spawn, type ChildProcessByStdio } from "node:child_process"
import { constants } from "node:fs"
import { access, mkdir, mkdtemp, rm } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import type { Readable } from "node:stream"
import { fileURLToPath } from "node:url"

export interface ProductionServer {
  baseURL: string
  setupToken: string
  stop: () => Promise<void>
}

const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url))
const binaryPath = join(repositoryRoot, "target", "release", "raindrop")
type ServerProcess = ChildProcessByStdio<null, Readable, Readable>

export async function startProductionServer(): Promise<ProductionServer> {
  await access(binaryPath, constants.X_OK).catch(() => {
    throw new Error("release server is missing; run the E2E prepare command")
  })
  const root = await mkdtemp(join(tmpdir(), "raindrop-e2e-"))
  const dataDir = join(root, "data")
  await mkdir(dataDir, { recursive: true })
  const port = await reservePort()
  const baseURL = `http://127.0.0.1:${port}`
  const child = spawn(binaryPath, [], {
    cwd: root,
    env: isolatedEnvironment(dataDir, port),
    stdio: ["ignore", "pipe", "pipe"],
  })
  let rejectSpawn: ((error: Error) => void) | undefined
  const onSpawnError = () => rejectSpawn?.(new Error("release server could not start"))
  const spawnFailure = new Promise<never>((_resolve, reject) => {
    rejectSpawn = reject
    child.once("error", onSpawnError)
  })

  try {
    const [setupToken] = await Promise.race([
      Promise.all([readSetupToken(child), waitUntilReady(baseURL, child)]),
      spawnFailure,
    ])
    return {
      baseURL,
      setupToken,
      stop: once(async () => {
        await stopProcess(child)
        await rm(root, { recursive: true, force: true })
      }),
    }
  } catch (error) {
    await stopProcess(child)
    await rm(root, { recursive: true, force: true })
    throw error
  } finally {
    child.removeListener("error", onSpawnError)
  }
}

function isolatedEnvironment(dataDir: string, port: number): NodeJS.ProcessEnv {
  const environment = Object.fromEntries(
    Object.entries(process.env).filter(([name]) => !name.startsWith("RAINDROP_")),
  )
  return {
    ...environment,
    RAINDROP_BIND: `127.0.0.1:${port}`,
    RAINDROP_DATA_DIR: dataDir,
    RUST_LOG: "error",
  }
}

async function reservePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (!address || typeof address === "string") {
        server.close()
        reject(new Error("could not reserve a local test port"))
        return
      }
      server.close((error) => (error ? reject(error) : resolve(address.port)))
    })
  })
}

function readSetupToken(child: ServerProcess): Promise<string> {
  return new Promise((resolve, reject) => {
    let settled = false
    const buffers = new Map<NodeJS.ReadableStream, string>([
      [child.stdout, ""],
      [child.stderr, ""],
    ])
    const finish = (result: { token?: string; error?: Error }) => {
      if (settled) return
      settled = true
      clearTimeout(timeout)
      for (const stream of buffers.keys()) stream.removeListener("data", onData)
      child.removeListener("error", onError)
      child.removeListener("exit", onExit)
      buffers.clear()
      if (result.token) resolve(result.token)
      else reject(result.error ?? new Error("server did not emit a setup token"))
    }
    const onData = function (this: NodeJS.ReadableStream, chunk: Buffer | string) {
      const next = `${buffers.get(this) ?? ""}${chunk.toString()}`
      const lines = next.split(/\r?\n/u)
      buffers.set(this, lines.pop() ?? "")
      for (const line of lines) {
        const match = /^Raindrop setup token: (.+)$/u.exec(line)
        if (match?.[1]) {
          finish({ token: match[1] })
          return
        }
      }
    }
    for (const stream of buffers.keys()) stream.on("data", onData)
    const onError = () => finish({ error: new Error("release server could not start") })
    const onExit = () =>
      finish({ error: new Error("server exited before setup was available") })
    child.once("error", onError)
    child.once("exit", onExit)
    const timeout = setTimeout(
      () => finish({ error: new Error("server setup token timed out") }),
      20_000,
    )
  })
}

async function waitUntilReady(
  baseURL: string,
  child: ServerProcess,
): Promise<void> {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    if (hasExited(child)) throw new Error("server exited before becoming ready")
    try {
      const response = await fetch(`${baseURL}/api/v1/health/live`, {
        signal: AbortSignal.timeout(1_000),
      })
      if (response.ok) return
    } catch {
      // The listener may not be bound yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error("server readiness timed out")
}

async function stopProcess(child: ServerProcess): Promise<void> {
  if (hasExited(child)) return
  await new Promise<void>((resolve) => {
    let settled = false
    let forceTimer: ReturnType<typeof setTimeout> | undefined
    let boundTimer: ReturnType<typeof setTimeout> | undefined
    const finish = () => {
      if (settled) return
      settled = true
      if (forceTimer) clearTimeout(forceTimer)
      if (boundTimer) clearTimeout(boundTimer)
      child.removeListener("exit", finish)
      resolve()
    }
    child.once("exit", finish)
    if (hasExited(child)) {
      finish()
      return
    }
    child.kill("SIGTERM")
    forceTimer = setTimeout(() => {
      if (!hasExited(child)) child.kill("SIGKILL")
    }, 2_000)
    boundTimer = setTimeout(finish, 4_000)
  })
}

function hasExited(child: ServerProcess): boolean {
  return child.pid === undefined || child.exitCode !== null || child.signalCode !== null
}

function once(action: () => Promise<void>): () => Promise<void> {
  let result: Promise<void> | undefined
  return () => (result ??= action())
}
