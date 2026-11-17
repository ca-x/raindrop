import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { SpeechSettingsPanel } from "./SpeechSettingsPanel"
import { stopAllSpeech } from "./api"
const config = {
  isEnabled: true,
  baseUrl: "https://speech.example/v1",
  model: "tts-1",
  voice: "zh-CN-YunyangNeural",
  speed: 1,
  revision: 7,
  hasApiKey: true,
  keyringAvailable: true,
}
let fetchMock: ReturnType<typeof vi.fn>
beforeEach(() => {
  activateLocale("en")
  fetchMock = vi.fn().mockImplementation((_path, init) => {
    if (init?.method === "PUT")
      return Promise.resolve(
        Response.json({ ...config, ...JSON.parse(init.body), revision: 8 }),
      )
    return Promise.resolve(Response.json(config))
  })
  vi.stubGlobal("fetch", fetchMock)
})
afterEach(() => vi.unstubAllGlobals())
function panel() {
  return render(
    <Providers>
      <SpeechSettingsPanel csrfToken="csrf" />
    </Providers>,
  )
}
it("persists the selected Edge voice without rotating the saved key", async () => {
  const user = userEvent.setup()
  panel()
  const voice = await screen.findByRole("combobox", { name: "Voice" })
  expect(voice).toHaveTextContent("云扬")
  await user.click(voice)
  await user.keyboard("{ArrowDown}{Enter}")
  fireEvent.change(
    screen.getByRole("textbox", {
      name: "Preview text (up to 200 characters)",
    }),
    { target: { value: "Preview text stays out of PUT" } },
  )
  await user.click(screen.getByRole("button", { name: "Save speech settings" }))
  await screen.findByText("Speech settings saved.")
  const body = JSON.parse(
    fetchMock.mock.calls.find(([, init]) => init.method === "PUT")![1].body,
  )
  expect(body).toEqual({
    isEnabled: true,
    baseUrl: config.baseUrl,
    model: "tts-1",
    voice: "zh-CN-XiaoxiaoNeural",
    speed: 1,
    revision: 7,
  })
  expect(screen.getByRole("combobox", { name: "Voice" })).toHaveTextContent(
    "晓晓",
  )
})
it("uses an arbitrary selected custom ID for the unsaved preview and revokes preview audio on unmount", async () => {
  const user = userEvent.setup()
  fetchMock.mockImplementation((path) =>
    path.endsWith("/test")
      ? Promise.resolve(
          new Response("mp3", { headers: { "content-type": "audio/mpeg" } }),
        )
      : Promise.resolve(Response.json({ ...config, voice: "my-custom-voice" })),
  )
  const pause = vi.fn()
  const load = vi.fn()
  vi.stubGlobal(
    "Audio",
    class {
      play = vi.fn().mockResolvedValue(undefined)
      pause = pause
      load = load
      removeAttribute = vi.fn()
    },
  )
  const revoke = vi.fn()
  const create = vi.fn().mockReturnValue("blob:preview")
  vi.spyOn(URL, "createObjectURL").mockImplementation(create)
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(revoke)
  const view = panel()
  const voice = await screen.findByRole("textbox", { name: "Custom voice ID" })
  fireEvent.change(voice, { target: { value: "custom-edited" } })
  fireEvent.change(
    screen.getByRole("textbox", {
      name: "Preview text (up to 200 characters)",
    }),
    { target: { value: "Hello" } },
  )
  expect(fetchMock).toHaveBeenCalledTimes(1)
  expect(screen.getByText(/may incur charges/u)).toBeVisible()
  await user.click(screen.getByRole("button", { name: "Test and play" }))
  await waitFor(() => expect(create).toHaveBeenCalledTimes(2))
  const call = fetchMock.mock.calls.find(([path]) => path.endsWith("/test"))!
  expect(JSON.parse(call[1].body)).toMatchObject({
    voice: "custom-edited",
    text: "Hello",
    revision: 7,
  })
  expect(JSON.parse(call[1].body)).not.toHaveProperty("apiKey")
  view.unmount()
  expect(call[1].signal.aborted).toBe(true)
  expect(pause).toHaveBeenCalled()
  expect(load).toHaveBeenCalled()
  expect(revoke).toHaveBeenCalledWith("blob:preview")
  vi.restoreAllMocks()
})
it("clears the key only after an explicit remove and save", async () => {
  const user = userEvent.setup()
  panel()
  await user.click(
    await screen.findByRole("button", { name: "Remove saved API Key" }),
  )
  await user.click(screen.getByRole("button", { name: "Save speech settings" }))
  await screen.findByText("Speech settings saved.")
  const body = JSON.parse(
    fetchMock.mock.calls.find(([, init]) => init.method === "PUT")![1].body,
  )
  expect(body.apiKey).toBeNull()
})
it("shows load failures rather than an endless spinner, with redacted bilingual feedback", async () => {
  activateLocale("zh-CN")
  fetchMock.mockRejectedValue(new Error("provider-secret-value"))
  panel()
  expect(await screen.findByText(/语音操作未完成/u)).toBeVisible()
  expect(screen.queryByText(/provider-secret-value/u)).not.toBeInTheDocument()
})
it("aborts a pending config read on logout", () => {
  fetchMock.mockReturnValue(new Promise(() => {}))
  panel()
  stopAllSpeech()
  expect(fetchMock.mock.calls[0]![1].signal.aborted).toBe(true)
})

it.each([
  "https://speech.example/v1/audio/speech",
  "https://speech.example/audio/speech/",
])(
  "rejects a speech operation URL instead of sending a config or preview: %s",
  async (baseUrl) => {
    const user = userEvent.setup()
    panel()
    fireEvent.change(
      await screen.findByRole("textbox", { name: "API base URL" }),
      { target: { value: baseUrl } },
    )
    fireEvent.change(
      screen.getByRole("textbox", {
        name: "Preview text (up to 200 characters)",
      }),
      { target: { value: "Hello" } },
    )
    await user.click(
      screen.getByRole("button", { name: "Save speech settings" }),
    )
    await user.click(screen.getByRole("button", { name: "Test and play" }))
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(screen.getByText(/Speech could not be completed/u)).toBeVisible()
  },
)
it("leaves origin-only normalization to the backend", async () => {
  const user = userEvent.setup()
  panel()
  fireEvent.change(
    await screen.findByRole("textbox", { name: "API base URL" }),
    { target: { value: "https://speech.example" } },
  )
  await user.click(screen.getByRole("button", { name: "Save speech settings" }))
  await screen.findByText("Speech settings saved.")
  expect(
    JSON.parse(
      fetchMock.mock.calls.find(([, init]) => init.method === "PUT")![1].body,
    ).baseUrl,
  ).toBe("https://speech.example")
})
it.each([
  [409, "TTS_CONFLICT"],
  [409, "TTS_DISABLED"],
  [400, "INVALID_INPUT"],
  [502, "UPSTREAM_AUTH"],
  [502, "UPSTREAM_INVALID"],
  [502, "UPSTREAM"],
  [504, "TIMEOUT"],
  [429, "RATE_LIMITED"],
  [503, "KEYRING_UNAVAILABLE"],
  [422, "INVALID_JSON"],
])(
  "redacts backend save error %s %s without retrying",
  async (status, code) => {
    const user = userEvent.setup()
    fetchMock.mockImplementation((_path, init) =>
      Promise.resolve(
        init?.method === "PUT"
          ? Response.json(
              { error: { code, message: "private-provider-value" } },
              { status: status as number },
            )
          : Response.json(config),
      ),
    )
    panel()
    await screen.findByRole("combobox", { name: "Voice" })
    await user.click(
      screen.getByRole("button", { name: "Save speech settings" }),
    )
    expect(
      await screen.findByText(/Speech could not be completed/u),
    ).toBeVisible()
    expect(
      screen.queryByText(/private-provider-value/u),
    ).not.toBeInTheDocument()
    expect(
      fetchMock.mock.calls.filter(([, init]) => init.method === "PUT"),
    ).toHaveLength(1)
  },
)
