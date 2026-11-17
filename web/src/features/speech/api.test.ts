import { afterEach, expect, it, vi } from "vitest"
import {
  getSpeechConfig,
  putSpeechConfig,
  testSpeech,
  planSpeech,
  speakChunk,
  type SpeechDraft,
} from "./api"
export const config = {
  isEnabled: true,
  baseUrl: "https://speech.example/v1",
  model: "tts-1",
  voice: "zh-CN-YunyangNeural",
  speed: 1,
  revision: 7,
  hasApiKey: true,
  keyringAvailable: true,
}
const draft: SpeechDraft = {
  isEnabled: true,
  baseUrl: config.baseUrl,
  model: config.model,
  voice: config.voice,
  speed: 1,
  revision: 7,
}
afterEach(() => vi.unstubAllGlobals())
it("uses direct configuration fields and preserves/clears credentials with CAS and CSRF", async () => {
  const fetch = vi
    .fn()
    .mockImplementation(() => Promise.resolve(Response.json(config)))
  vi.stubGlobal("fetch", fetch)
  expect(await getSpeechConfig()).toEqual(config)
  await putSpeechConfig("csrf", draft)
  let init = fetch.mock.calls[1]![1] as RequestInit
  expect(init.method).toBe("PUT")
  expect(new Headers(init.headers).get("x-csrf-token")).toBe("csrf")
  expect(JSON.parse(init.body as string)).toEqual(draft)
  expect(init.body).not.toContain("apiKey")
  await putSpeechConfig("csrf", { ...draft, apiKey: null })
  init = fetch.mock.calls[2]![1] as RequestInit
  expect(JSON.parse(init.body as string).apiKey).toBeNull()
})
it("sends the exact selected voice and unsaved draft for an explicit audio preview", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValue(
      new Response("mp3", { headers: { "content-type": "audio/mpeg" } }),
    )
  vi.stubGlobal("fetch", fetch)
  expect(
    (await testSpeech("csrf", { ...draft, voice: "custom-id" }, "Hello")).type,
  ).toBe("audio/mpeg")
  expect(JSON.parse(fetch.mock.calls[0]![1].body)).toEqual({
    ...draft,
    voice: "custom-id",
    text: "Hello",
  })
})
it("plans authorized entries and speaks snapshots without substituted article text", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(
      Response.json({
        entryId: "entry/1",
        contentHash: "hash",
        chunks: ["first"],
      }),
    )
    .mockResolvedValueOnce(
      new Response("mp3", { headers: { "content-type": "audio/mpeg" } }),
    )
  vi.stubGlobal("fetch", fetch)
  const plan = await planSpeech("entry/1", "csrf")
  await speakChunk(plan.entryId, "csrf", plan.contentHash, 0)
  expect(fetch.mock.calls[0]![0]).toContain("entry%2F1/plan")
  expect(JSON.parse(fetch.mock.calls[1]![1].body)).toEqual({
    contentHash: "hash",
    chunkIndex: 0,
  })
})
it("rejects malformed configs and non audio responses without retrying", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(Response.json({ config }))
    .mockResolvedValueOnce(Response.json({ error: "secret" }))
  vi.stubGlobal("fetch", fetch)
  await expect(getSpeechConfig()).rejects.toThrow("Invalid server response")
  await expect(testSpeech("csrf", draft, "Hello")).rejects.toThrow(
    "Invalid server response",
  )
  expect(fetch).toHaveBeenCalledTimes(2)
})
