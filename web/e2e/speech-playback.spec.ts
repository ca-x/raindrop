import { readFile } from "node:fs/promises"
import { expect, test } from "@playwright/test"

import { completeSetup, createCredentials } from "./support/app"
import { installReaderApiFixture, readerIds } from "./support/readerApiFixture"
import {
  startProductionServer,
  type ProductionServer,
} from "./support/productionServer"

interface MediaEventRecord {
  type: string
  source: string
  duration: number
  currentTime: number
  trusted: boolean
}
interface SpeechMediaAudit {
  elements: number
  events: MediaEventRecord[]
  cspViolations: string[]
}

declare global {
  interface Window {
    speechMediaAudit: SpeechMediaAudit
  }
}

let server: ProductionServer

test.beforeEach(async () => {
  server = await startProductionServer()
})
test.afterEach(async () => {
  await server?.stop()
})

test("production CSP permits gesture-unlocked native audio to play two sequential speech chunks", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(90_000)
  const launchArguments = testInfo.project.use.launchOptions?.args ?? []
  expect(
    launchArguments.some((argument) =>
      argument.startsWith("--autoplay-policy"),
    ),
  ).toBe(false)
  const pageErrors: string[] = []
  page.on("pageerror", (error) => pageErrors.push(error.message))
  await completeSetup(page, server, createCredentials())
  await installReaderApiFixture(page)
  const mp3 = await readFile(
    new URL("../../desktop/src/fixtures/silence.mp3", import.meta.url),
  )
  await page.addInitScript(() => {
    const audit: SpeechMediaAudit = {
      elements: 0,
      events: [],
      cspViolations: [],
    }
    window.speechMediaAudit = audit
    document.addEventListener("securitypolicyviolation", (event) => {
      audit.cspViolations.push(event.violatedDirective)
    })
    const NativeAudio = window.Audio
    const ObservedAudio = function (source?: string) {
      const audio =
        source === undefined ? new NativeAudio() : new NativeAudio(source)
      audit.elements++
      for (const type of ["playing", "ended", "error"]) {
        audio.addEventListener(type, (event) => {
          audit.events.push({
            type,
            source: audio.currentSrc || audio.src,
            duration: audio.duration,
            currentTime: audio.currentTime,
            trusted: event.isTrusted,
          })
        })
      }
      return audio
    }
    ObservedAudio.prototype = NativeAudio.prototype
    Object.setPrototypeOf(ObservedAudio, NativeAudio)
    window.Audio = ObservedAudio as unknown as typeof Audio
  })
  const requests: Array<{ chunkIndex: number; endedBeforeRequest: number }> = []
  await page.route("**/api/v1/plugins/tts**", async (route) => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    if (path === "/api/v1/plugins/tts" && request.method() === "GET") {
      await route.fulfill({
        json: {
          isEnabled: true,
          baseUrl: "https://speech.example/v1",
          model: "tts-1",
          voice: "zh-CN-YunyangNeural",
          speed: 1,
          hasApiKey: false,
          keyringAvailable: true,
          revision: 0,
        },
      })
      return
    }
    expect(request.method()).toBe("POST")
    expect(request.headers()["x-csrf-token"]).toBeTruthy()
    if (path === `/api/v1/plugins/tts/entries/${readerIds.firstEntry}/plan`) {
      await route.fulfill({
        json: {
          entryId: readerIds.firstEntry,
          contentHash: "browser-speech-snapshot",
          chunks: ["First clip", "Second clip"],
        },
      })
      return
    }
    expect(path).toBe(
      `/api/v1/plugins/tts/entries/${readerIds.firstEntry}/speak`,
    )
    const body = request.postDataJSON() as {
      contentHash: string
      chunkIndex: number
    }
    expect(body).toEqual({
      contentHash: "browser-speech-snapshot",
      chunkIndex: requests.length,
    })
    const endedBeforeRequest = await page.evaluate(
      () =>
        window.speechMediaAudit.events.filter((event) => event.type === "ended")
          .length,
    )
    requests.push({ chunkIndex: body.chunkIndex, endedBeforeRequest })
    await route.fulfill({ contentType: "audio/mpeg", body: mp3 })
  })
  const documentResponse = await page.goto(
    `${server.baseURL}/reader/feed/${readerIds.feedA}/entry/${readerIds.firstEntry}`,
    { waitUntil: "domcontentloaded" },
  )
  expect(documentResponse).not.toBeNull()
  const policy = documentResponse!.headers()["content-security-policy"] ?? ""
  const mediaDirective = policy
    .split(";")
    .map((directive) => directive.trim().split(/\s+/u))
    .find((directive) => directive[0] === "media-src")
  expect(mediaDirective).toContain("blob:")
  expect(mediaDirective).not.toContain("data:")
  expect(mediaDirective).not.toContain("*")
  const playButton = page.getByRole("button", {
    name: "Read article aloud",
    exact: true,
  })
  await expect(playButton).toBeEnabled()
  await playButton.click()
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.speechMediaAudit.events.filter(
            (event) => event.type === "ended",
          ).length,
      ),
    )
    .toBe(2)
  await expect(playButton).toBeEnabled()
  await expect(
    page
      .getByRole("group", { name: "Article speech controls" })
      .getByText("Ready", { exact: true }),
  ).toHaveClass("reader-visually-hidden")
  await expect(
    page.getByRole("button", { name: "Stop speech", exact: true }),
  ).toBeDisabled()
  expect(requests).toEqual([
    { chunkIndex: 0, endedBeforeRequest: 0 },
    { chunkIndex: 1, endedBeforeRequest: 1 },
  ])
  const audit = await page.evaluate(() => window.speechMediaAudit)
  expect(audit.elements).toBe(1)
  expect(audit.cspViolations).toEqual([])
  expect(audit.events.filter((event) => event.type === "error")).toEqual([])
  const played = audit.events.filter((event) => event.type === "playing")
  const ended = audit.events.filter((event) => event.type === "ended")
  expect(played).toHaveLength(3)
  expect(new Set(played.map((event) => event.source)).size).toBe(3)
  expect(ended.map((event) => event.source)).toEqual(
    played.slice(1).map((event) => event.source),
  )
  for (const event of ended) {
    expect(event.trusted).toBe(true)
    expect(event.duration).toBeGreaterThan(0)
    expect(event.currentTime).toBeGreaterThan(0)
  }
  expect(pageErrors).toEqual([])
})
