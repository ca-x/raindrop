import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { SpeechControls } from "./SpeechControls"
import { configChanged, stopAllSpeech } from "./api"
let speechEnabled: boolean
let audios: FakeAudio[]
class FakeAudio {
  onended: (() => void) | null = null
  onerror: (() => void) | null = null
  play = vi.fn().mockResolvedValue(undefined)
  pause = vi.fn()
  load = vi.fn()
  removeAttribute = vi.fn()
  constructor() {
    audios.push(this)
  }
}
let fetchMock: ReturnType<typeof vi.fn>
let revoke: ReturnType<typeof vi.fn>
beforeEach(() => {
  activateLocale("en")
  speechEnabled = true
  audios = []
  vi.stubGlobal("Audio", FakeAudio)
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:audio")
  revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {})
  fetchMock = vi.fn().mockImplementation((path) => {
    if (path.endsWith("/plan"))
      return Promise.resolve(
        Response.json({
          entryId: "one",
          contentHash: "hash",
          chunks: ["first", "second"],
        }),
      )
    if (path.endsWith("/speak"))
      return Promise.resolve(
        new Response("mp3", { headers: { "content-type": "audio/mpeg" } }),
      )
    return Promise.resolve(
      Response.json({
        isEnabled: speechEnabled,
        baseUrl: "https://speech.example",
        model: "tts",
        voice: "alloy",
        speed: 1,
        hasApiKey: false,
        keyringAvailable: true,
        revision: 0,
      }),
    )
  })
  vi.stubGlobal("fetch", fetchMock)
})
afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})
function content(entryId = "one") {
  return (
    <Providers>
      <SpeechControls entryId={entryId} csrfToken="csrf" />
    </Providers>
  )
}
async function start() {
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Read article aloud" }),
    ).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole("button", { name: "Read article aloud" }))
  await screen.findByRole("button", { name: "Pause speech" })
}
it("removes the toolbar when speech is turned off while reading", async () => {
  render(content())
  await screen.findByRole("button", { name: "Read article aloud" })
  speechEnabled = false
  await act(async () => configChanged())
  expect(screen.queryByRole("button", { name: "Read article aloud" })).toBeNull()
  expect(document.querySelector(".speech-controls")).toBeNull()
})
it("provides accessible icon controls and requests the next chunk only after audio ends", async () => {
  render(content())
  await start()
  expect(
    fetchMock.mock.calls.filter(([path]) => path.endsWith("/speak")),
  ).toHaveLength(1)
  fireEvent.click(screen.getByRole("button", { name: "Pause speech" }))
  expect(screen.getByText("Paused")).toHaveAttribute("role", "status")
  expect(screen.getByText("Paused")).toHaveClass("speech-feedback")
  fireEvent.click(screen.getByRole("button", { name: "Resume speech" }))
  await screen.findByRole("button", { name: "Pause speech" })
  act(() => audios[0]!.onended?.())
  await waitFor(() =>
    expect(
      fetchMock.mock.calls.filter(([path]) => path.endsWith("/speak")),
    ).toHaveLength(2),
  )
  expect(audios).toHaveLength(1)
  expect(
    JSON.parse(
      fetchMock.mock.calls.filter(([path]) => path.endsWith("/speak"))[1]![1]
        .body,
    ),
  ).toEqual({ contentHash: "hash", chunkIndex: 1 })
  fireEvent.click(screen.getByRole("button", { name: "Stop speech" }))
  expect(revoke).toHaveBeenCalledTimes(3)
})
it.each(["entry", "unmount", "logout"])(
  "cancels audio and synthesis on %s",
  async (cause) => {
    const view = render(content())
    await start()
    const speak = fetchMock.mock.calls.find(([path]) =>
      path.endsWith("/speak"),
    )!
    if (cause === "entry") view.rerender(content("two"))
    else if (cause === "unmount") view.unmount()
    else act(() => stopAllSpeech())
    expect(speak[1].signal.aborted).toBe(true)
    expect(audios[0]!.pause).toHaveBeenCalled()
    expect(revoke).toHaveBeenCalledWith("blob:audio")
  },
)
it("redacts provider failures and does not automatically repeat paid requests", async () => {
  fetchMock.mockImplementation((path) => {
    if (path.endsWith("/plan"))
      return Promise.reject(new Error("private-api-key"))
    return Promise.resolve(
      Response.json({
        isEnabled: true,
        baseUrl: "https://speech.example",
        model: "tts",
        voice: "alloy",
        speed: 1,
        hasApiKey: false,
        keyringAvailable: true,
        revision: 0,
      }),
    )
  })
  render(content())
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Read article aloud" }),
    ).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole("button", { name: "Read article aloud" }))
  expect(await screen.findByRole("alert")).not.toHaveTextContent(
    "private-api-key",
  )
  expect(
    fetchMock.mock.calls.filter(([path]) => path.endsWith("/plan")),
  ).toHaveLength(1)
})
