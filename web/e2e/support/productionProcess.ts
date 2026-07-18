import type { ChildProcessByStdio } from "node:child_process"
import { createConnection } from "node:net"
import type { Readable } from "node:stream"

export interface ServerTimeouts {
  setupMs: number
  readinessMs: number
  probeMs: number
  termMs: number
  killMs: number
  listenerMs: number
}

export interface ProcessLifecycle {
  closed: Promise<void>
  isClosed: () => boolean
  spawnFailure: Promise<never>
}

export type ServerProcess = ChildProcessByStdio<null, Readable, Readable>

export const defaultTimeouts: ServerTimeouts = {
  setupMs: 20_000,
  readinessMs: 20_000,
  probeMs: 1_000,
  termMs: 2_000,
  killMs: 2_000,
  listenerMs: 2_000,
}

export function trackProcess(child: ServerProcess): ProcessLifecycle {
  let closed = false
  const closedPromise = new Promise<void>((resolve) => {
    child.once("close", () => {
      closed = true
      resolve()
    })
  })
  const spawnFailure = new Promise<never>((_resolve, reject) => {
    child.once("error", () => reject(new Error("release server could not start")))
  })
  return {
    closed: closedPromise,
    isClosed: () => closed,
    spawnFailure,
  }
}

export function drainProcessOutput(
  child: ServerProcess,
  lifecycle: ProcessLifecycle,
  timeoutMs: number,
): Promise<string> {
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
      child.removeListener("error", onError)
      buffers.clear()
      if (result.token) resolve(result.token)
      else reject(result.error ?? new Error("server did not emit a setup token"))
    }
    const onData = function (this: NodeJS.ReadableStream, chunk: Buffer | string) {
      if (settled) return
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
    const onError = () => finish({ error: new Error("release server could not start") })
    for (const stream of buffers.keys()) stream.on("data", onData)
    child.once("error", onError)
    void lifecycle.closed.then(() => {
      for (const stream of [child.stdout, child.stderr]) {
        stream.removeListener("data", onData)
      }
      finish({ error: new Error("server exited before setup was available") })
    })
    const timeout = setTimeout(
      () => finish({ error: new Error("server setup token timed out") }),
      timeoutMs,
    )
  })
}

export async function waitUntilReady(
  baseURL: string,
  lifecycle: ProcessLifecycle,
  timeouts: ServerTimeouts,
): Promise<void> {
  const deadline = Date.now() + timeouts.readinessMs
  while (Date.now() < deadline) {
    if (lifecycle.isClosed()) throw new Error("server exited before becoming ready")
    try {
      const response = await fetch(`${baseURL}/api/v1/health/live`, {
        signal: AbortSignal.timeout(timeouts.probeMs),
      })
      if (response.ok) return
    } catch {
      // The listener may not be bound yet.
    }
    await delay(25)
  }
  throw new Error("server readiness timed out")
}

export async function stopProcess(
  child: ServerProcess,
  lifecycle: ProcessLifecycle,
  baseURL: string,
  timeouts: ServerTimeouts,
): Promise<void> {
  if (!lifecycle.isClosed()) {
    if (child.pid !== undefined) child.kill("SIGTERM")
    if (!(await closesWithin(lifecycle, timeouts.termMs))) {
      if (child.pid !== undefined) child.kill("SIGKILL")
      if (!(await closesWithin(lifecycle, timeouts.killMs))) {
        throw new Error("release server could not be reaped")
      }
    }
  }
  await waitUntilListenerClosed(baseURL, timeouts)
}

async function closesWithin(
  lifecycle: ProcessLifecycle,
  timeoutMs: number,
): Promise<boolean> {
  if (lifecycle.isClosed()) return true
  return new Promise((resolve) => {
    const timeout = setTimeout(() => resolve(false), timeoutMs)
    void lifecycle.closed.then(() => {
      clearTimeout(timeout)
      resolve(true)
    })
  })
}

async function waitUntilListenerClosed(
  baseURL: string,
  timeouts: ServerTimeouts,
): Promise<void> {
  const url = new URL(baseURL)
  const port = Number(url.port)
  const deadline = Date.now() + timeouts.listenerMs
  while (Date.now() < deadline) {
    if (!(await listenerIsOpen(url.hostname, port, timeouts.probeMs))) return
    await delay(25)
  }
  throw new Error("release server listener did not close")
}

function listenerIsOpen(host: string, port: number, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host, port })
    let settled = false
    const finish = (open: boolean) => {
      if (settled) return
      settled = true
      socket.destroy()
      resolve(open)
    }
    socket.once("connect", () => finish(true))
    socket.once("error", () => finish(false))
    socket.setTimeout(timeoutMs, () => finish(false))
  })
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds))
}
