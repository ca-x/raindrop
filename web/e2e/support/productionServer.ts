import { spawn } from "node:child_process"
import { constants } from "node:fs"
import { access, mkdir, mkdtemp, rm } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

import {
  defaultTimeouts,
  drainProcessOutput,
  stopProcess,
  trackProcess,
  waitUntilReady,
  type ProcessLifecycle,
  type ServerProcess,
  type ServerTimeouts,
} from "./productionProcess"

export interface ProductionServer {
  baseURL: string
  setupToken: string
  stop: () => Promise<void>
}

export interface StartProductionServerOptions {
  binaryPath?: string
  temporaryRootParent?: string
  environment?: NodeJS.ProcessEnv
  timeouts?: Partial<ServerTimeouts>
}

const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url))
const defaultBinaryPath = join(repositoryRoot, "target", "release", "raindrop")

export async function startProductionServer(
  options: StartProductionServerOptions = {},
): Promise<ProductionServer> {
  const executable = options.binaryPath ?? defaultBinaryPath
  const timeouts = { ...defaultTimeouts, ...options.timeouts }
  let root: string | undefined
  let child: ServerProcess | undefined
  let lifecycle: ProcessLifecycle | undefined
  let baseURL: string | undefined
  let cleanupTransferred = false

  try {
    await access(executable, constants.X_OK).catch(() => {
      throw new Error("release server is missing; run the E2E prepare command")
    })
    root = await mkdtemp(
      join(options.temporaryRootParent ?? tmpdir(), "raindrop-e2e-"),
    )
    const dataDir = join(root, "data")
    await mkdir(dataDir, { recursive: true })
    const port = await reservePort()
    baseURL = `http://127.0.0.1:${port}`
    try {
      child = spawn(executable, [], {
        cwd: root,
        env: isolatedEnvironment(options.environment ?? process.env, dataDir, port),
        stdio: ["ignore", "pipe", "pipe"],
      })
    } catch {
      throw new Error("release server could not start")
    }
    lifecycle = trackProcess(child)
    const setupToken = await Promise.race([
      Promise.all([
        drainProcessOutput(child, lifecycle, timeouts.setupMs),
        waitUntilReady(baseURL, lifecycle, timeouts),
      ]).then(([token]) => token),
      lifecycle.spawnFailure,
    ])

    cleanupTransferred = true
    const ownedRoot = root
    const ownedChild = child
    const ownedLifecycle = lifecycle
    const ownedBaseURL = baseURL
    return {
      baseURL,
      setupToken,
      stop: once(async () => {
        try {
          await stopProcess(ownedChild, ownedLifecycle, ownedBaseURL, timeouts)
        } finally {
          await removeTemporaryRoot(ownedRoot)
        }
      }),
    }
  } catch (error) {
    let failure = stableFailure(error)
    if (child && lifecycle && baseURL) {
      try {
        await stopProcess(child, lifecycle, baseURL, timeouts)
      } catch (cleanupError) {
        failure = stableFailure(cleanupError)
      }
    }
    throw failure
  } finally {
    if (!cleanupTransferred && root) await removeTemporaryRoot(root)
  }
}

function isolatedEnvironment(
  source: NodeJS.ProcessEnv,
  dataDir: string,
  port: number,
): NodeJS.ProcessEnv {
  const environment = Object.fromEntries(
    Object.entries(source).filter(([name]) => !name.startsWith("RAINDROP_")),
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
    const fail = () => reject(new Error("could not reserve a local test port"))
    server.once("error", fail)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (!address || typeof address === "string") {
        server.close(() => fail())
        return
      }
      const port = address.port
      server.close((error) => {
        if (error) fail()
        else resolve(port)
      })
    })
  })
}

async function removeTemporaryRoot(root: string): Promise<void> {
  try {
    await rm(root, { recursive: true, force: true })
  } catch {
    throw new Error("temporary test data could not be removed")
  }
}

function stableFailure(error: unknown): Error {
  if (error instanceof Error && stableMessages.has(error.message)) return error
  return new Error("production server fixture setup failed")
}

function once(action: () => Promise<void>): () => Promise<void> {
  let result: Promise<void> | undefined
  return () => (result ??= action())
}

const stableMessages = new Set([
  "release server is missing; run the E2E prepare command",
  "could not reserve a local test port",
  "release server could not start",
  "server did not emit a setup token",
  "server exited before setup was available",
  "server setup token timed out",
  "server exited before becoming ready",
  "server readiness timed out",
  "release server could not be reaped",
  "release server listener did not close",
  "temporary test data could not be removed",
])
