import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { expect, test } from "@playwright/test"

import { startProductionServer } from "./support/productionServer"

let sandbox: string
let temporaryRootParent: string
const remainingPids = new Set<number>()

test.beforeEach(async () => {
  sandbox = await mkdtemp(join(tmpdir(), "raindrop-fixture-test-"))
  temporaryRootParent = join(sandbox, "roots")
  await mkdir(temporaryRootParent)
})

test.afterEach(async () => {
  for (const pid of remainingPids) {
    try {
      process.kill(pid, "SIGKILL")
    } catch {
      // The expected cleanup path already reaped it.
    }
    await expectProcessGone(pid)
  }
  remainingPids.clear()
  await rm(sandbox, { recursive: true, force: true })
})

test("spawn failures are stable, redacted, and remove the temporary root", async () => {
  const executable = await fakeExecutable(
    "missing-interpreter",
    "#!/definitely/missing/node\n",
  )
  const secret = "spawn-secret-must-not-leak"

  const error = await rejectionOf(
    startProductionServer({
      binaryPath: executable,
      temporaryRootParent,
      environment: { ...process.env, FIXTURE_SECRET: secret },
      timeouts: shortTimeouts,
    }),
  )

  expect(error.message).toBe("release server could not start")
  expect(error.message).not.toContain(secret)
  await expect(readdir(temporaryRootParent)).resolves.toEqual([])
})

test("readiness failures reap the process without leaking its setup token", async () => {
  const pidFile = join(sandbox, "readiness.pid")
  const secret = "readiness-secret-must-not-leak"
  const executable = await fakeExecutable(
    "never-ready",
    `#!/usr/bin/env node
const fs = require("node:fs")
fs.writeFileSync(process.env.FIXTURE_PID_FILE, String(process.pid))
process.stdout.write("Raindrop setup token: " + process.env.FIXTURE_SECRET + "\\n")
process.on("SIGTERM", () => process.exit(0))
setInterval(() => {}, 1_000)
`,
  )

  const error = await rejectionOf(
    startProductionServer({
      binaryPath: executable,
      temporaryRootParent,
      environment: {
        ...process.env,
        FIXTURE_PID_FILE: pidFile,
        FIXTURE_SECRET: secret,
      },
      timeouts: shortTimeouts,
    }),
  )

  expect(error.message).toBe("server readiness timed out")
  expect(error.message).not.toContain(secret)
  const pid = Number(await readFile(pidFile, "utf8"))
  await expectProcessGone(pid)
  await expect(readdir(temporaryRootParent)).resolves.toEqual([])
})

test("shutdown escalates from TERM to KILL and confirms the listener closed", async () => {
  const pidFile = join(sandbox, "server.pid")
  const executable = await fakeExecutable(
    "ignores-term",
    `#!/usr/bin/env node
const fs = require("node:fs")
const http = require("node:http")
const address = process.env.RAINDROP_BIND.split(":")
const server = http.createServer((_request, response) => {
  response.writeHead(200, { "content-type": "application/json" })
  response.end("{}")
})
fs.writeFileSync(process.env.FIXTURE_PID_FILE, String(process.pid))
process.on("SIGTERM", () => {})
server.listen(Number(address[1]), address[0], () => {
  process.stdout.write("Raindrop setup token: " + process.env.FIXTURE_SECRET + "\\n")
})
`,
  )
  const server = await startProductionServer({
    binaryPath: executable,
    temporaryRootParent,
    environment: {
      ...process.env,
      FIXTURE_PID_FILE: pidFile,
      FIXTURE_SECRET: "term-secret-must-not-leak",
    },
    timeouts: shortTimeouts,
  })
  const pid = Number(await readFile(pidFile, "utf8"))

  await expect(server.stop()).resolves.toBeUndefined()
  await expectProcessGone(pid)
  await expect(readdir(temporaryRootParent)).resolves.toEqual([])
})

test("shutdown reports a stable error when another process keeps the listener open", async () => {
  const grandchildPidFile = join(sandbox, "listener.pid")
  const executable = await fakeExecutable(
    "leaves-listener",
    `#!/usr/bin/env node
const fs = require("node:fs")
const { spawn } = require("node:child_process")
const program = [
  'const http = require("node:http")',
  'const fs = require("node:fs")',
  'const address = process.env.RAINDROP_BIND.split(":")',
  'fs.writeFileSync(process.env.FIXTURE_PID_FILE, String(process.pid))',
  'http.createServer((_request, response) => { response.writeHead(200); response.end("{}") }).listen(Number(address[1]), address[0])',
].join(";")
const child = spawn(process.execPath, ["-e", program], {
  detached: true,
  env: process.env,
  stdio: "ignore",
})
child.unref()
setTimeout(() => {
  process.stdout.write("Raindrop setup token: " + process.env.FIXTURE_SECRET + "\\n")
}, 50)
setInterval(() => {}, 1_000)
`,
  )
  const secret = "listener-secret-must-not-leak"
  const server = await startProductionServer({
    binaryPath: executable,
    temporaryRootParent,
    environment: {
      ...process.env,
      FIXTURE_PID_FILE: grandchildPidFile,
      FIXTURE_SECRET: secret,
    },
    timeouts: shortTimeouts,
  })
  const grandchildPid = Number(await readFile(grandchildPidFile, "utf8"))
  remainingPids.add(grandchildPid)

  const error = await rejectionOf(server.stop())

  expect(error.message).toBe("release server listener did not close")
  expect(error.message).not.toContain(secret)
  await expect(readdir(temporaryRootParent)).resolves.toEqual([])
})

async function fakeExecutable(name: string, contents: string): Promise<string> {
  const path = join(sandbox, name)
  await writeFile(path, contents)
  await chmod(path, 0o700)
  return path
}

async function rejectionOf(promise: Promise<unknown>): Promise<Error> {
  try {
    await promise
  } catch (error) {
    return error instanceof Error ? error : new Error("non-error rejection")
  }
  throw new Error("expected the promise to reject")
}

async function expectProcessGone(pid: number): Promise<void> {
  await expect
    .poll(() => {
      try {
        process.kill(pid, 0)
        return false
      } catch {
        return true
      }
    })
    .toBe(true)
}

const shortTimeouts = {
  setupMs: 750,
  readinessMs: 400,
  probeMs: 100,
  termMs: 100,
  killMs: 500,
  listenerMs: 250,
}
