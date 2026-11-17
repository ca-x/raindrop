import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { SpeechPlayback } from "./playback"
class FakeAudio {
  onended: (() => void) | null = null
  onerror: (() => void) | null = null
  paused = true
  play = vi.fn().mockImplementation(() => {
    this.paused = false
    return Promise.resolve()
  })
  pause = vi.fn().mockImplementation(() => {
    this.paused = true
  })
  load = vi.fn()
  removeAttribute = vi.fn()
  constructor(public src = "") {
    audios.push(this)
  }
}
let audios: FakeAudio[]
let revoke: ReturnType<typeof vi.fn>
beforeEach(() => {
  audios = []
  vi.stubGlobal("Audio", FakeAudio)
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => `blob:${audios.length}`),
    revokeObjectURL: (revoke = vi.fn()),
  })
})
afterEach(() => vi.unstubAllGlobals())
const tick = async () => {
  for (let iteration = 0; iteration < 10; iteration++) await Promise.resolve()
}
it("synthesizes sequentially, pauses/resumes, and releases every blob", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  const synth = vi.fn().mockResolvedValue(new Blob())
  const done = player.run(async function* () {
    yield await synth(0)
    yield await synth(1)
  })
  await tick()
  expect(synth).toHaveBeenCalledTimes(1)
  player.pause()
  expect(audios[0]!.pause).toHaveBeenCalled()
  await player.resume()
  expect(audios[0]!.play).toHaveBeenCalledTimes(3)
  audios[0]!.onended?.()
  await tick()
  expect(synth).toHaveBeenCalledTimes(2)
  expect(audios).toHaveLength(1)
  audios[0]!.onended?.()
  await done
  expect(revoke).toHaveBeenCalledTimes(3)
  expect(state).toHaveBeenLastCalledWith("idle")
})
it("aborts pending synthesis and ignores late audio", async () => {
  const player = new SpeechPlayback(vi.fn())
  let signal: AbortSignal | undefined
  let resolve!: (blob: Blob) => void
  const done = player.run(async function* (abortSignal) {
    signal = abortSignal
    yield await new Promise<Blob>((complete) => {
      resolve = complete
    })
  })
  await tick()
  player.stop()
  expect(signal!.aborted).toBe(true)
  resolve(new Blob())
  await done
  expect(audios).toHaveLength(1)
  expect(audios[0]!.play).toHaveBeenCalledOnce()
})
it("does not let a stopped sequence release its replacement audio", async () => {
  const player = new SpeechPlayback(vi.fn())
  const first = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  const second = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  await first
  expect(audios[1]!.pause).toHaveBeenCalledOnce()
  audios[1]!.onended?.()
  await second
})
it("redacts playback errors and never retries synthesis", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  const synth = vi.fn().mockRejectedValue(new Error("private provider details"))
  await player.run(async function* () {
    yield await synth()
  })
  expect(synth).toHaveBeenCalledOnce()
  expect(state).toHaveBeenLastCalledWith("error")
})
it("does not pause or resume while synthesis has no audio", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  let resolve!: (blob: Blob) => void
  const done = player.run(async function* () {
    yield await new Promise<Blob>((complete) => {
      resolve = complete
    })
  })
  await tick()
  player.pause()
  await player.resume()
  expect(state).toHaveBeenLastCalledWith("loading")
  expect(state).not.toHaveBeenCalledWith("paused")
  player.stop()
  resolve(new Blob())
  await done
})
it("keeps paused state when an earlier play promise resolves late", async () => {
  const state = vi.fn()
  let resolvePlay!: () => void
  const pendingPlay = new Promise<void>((resolve) => {
    resolvePlay = resolve
  })
  const player = new SpeechPlayback(state)
  const done = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  audios[0]!.play.mockReturnValueOnce(pendingPlay)
  player.pause()
  const resume = player.resume()
  player.pause()
  resolvePlay()
  await resume
  expect(state).toHaveBeenLastCalledWith("paused")
  player.stop()
  await done
})
it.each(["before", "after"])(
  "waits for resume when pause occurs %s an ended callback",
  async (timing) => {
    const state = vi.fn()
    const synth = vi.fn().mockResolvedValue(new Blob())
    const player = new SpeechPlayback(state)
    const done = player.run(async function* () {
      yield await synth(0)
      yield await synth(1)
    })
    await tick()
    if (timing === "before") player.pause()
    audios[0]!.onended?.()
    if (timing === "after") player.pause()
    await tick()
    expect(state).toHaveBeenLastCalledWith("paused")
    expect(synth).toHaveBeenCalledTimes(1)
    await player.resume()
    await tick()
    expect(synth).toHaveBeenCalledTimes(2)
    player.stop()
    await done
  },
)
it("surfaces autoplay denial without reporting playing or retrying", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      override play = vi
        .fn()
        .mockRejectedValue(
          new DOMException("Gesture required", "NotAllowedError"),
        )
    },
  )
  const synth = vi.fn().mockResolvedValue(new Blob())
  await player.run(async function* () {
    yield await synth()
  })
  expect(state).not.toHaveBeenCalledWith("playing")
  expect(state).toHaveBeenLastCalledWith("error")
  expect(synth).not.toHaveBeenCalled()
  expect(revoke).toHaveBeenCalledOnce()
})

it("ignores a delayed initial play completion and its pause-induced rejection", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  let rejectPlay!: (error: unknown) => void
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      override play = vi
        .fn()
        .mockResolvedValueOnce(undefined)
        .mockImplementation(
          () =>
            new Promise<void>((_resolve, reject) => {
              rejectPlay = reject
            }),
        )
    },
  )
  const done = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  player.pause()
  rejectPlay(new DOMException("Interrupted by pause", "AbortError"))
  await tick()
  expect(state).toHaveBeenLastCalledWith("paused")
  expect(state).not.toHaveBeenCalledWith("error")
  player.stop()
  await done
})
it("does not announce playing when the browser remains paused after play resolves", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      override play = vi.fn().mockResolvedValue(undefined)
    },
  )
  const done = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  expect(state).not.toHaveBeenCalledWith("playing")
  player.stop()
  await done
})
it("ignores a stale ended callback from the replaced audio", async () => {
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  const first = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  const staleEnded = audios[0]!.onended!
  const second = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  staleEnded()
  player.pause()
  await player.resume()
  expect(audios[1]!.play).toHaveBeenCalledTimes(3)
  expect(state).toHaveBeenLastCalledWith("playing")
  player.stop()
  await Promise.all([first, second])
})

it("unlocks audible playback synchronously in the gesture and reuses that element for every chunk", async () => {
  let gestureActive = true
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      muted = false
      unlocked = false
      override play = vi.fn().mockImplementation(() => {
        if (gestureActive && !this.muted) this.unlocked = true
        if (!this.unlocked)
          return Promise.reject(
            new DOMException("Gesture required", "NotAllowedError"),
          )
        this.paused = false
        return Promise.resolve()
      })
    },
  )
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  const synth = vi
    .fn()
    .mockResolvedValue(new Blob(["mp3"], { type: "audio/mpeg" }))
  const done = player.run(async function* () {
    yield await synth(0)
    yield await synth(1)
  })
  expect(audios).toHaveLength(1)
  expect(audios[0]!.play).toHaveBeenCalledOnce()
  expect(synth).not.toHaveBeenCalled()
  expect(state).not.toHaveBeenCalledWith("playing")
  const unlockBlob = vi.mocked(URL.createObjectURL).mock.calls[0]![0] as Blob
  expect(unlockBlob.type).toBe("audio/wav")
  expect(unlockBlob.size).toBe(364)
  gestureActive = false
  await tick()
  expect(state).toHaveBeenLastCalledWith("playing")
  expect(audios[0]!.play).toHaveBeenCalledTimes(2)
  const staleEnded = audios[0]!.onended!
  staleEnded()
  await tick()
  expect(synth).toHaveBeenCalledTimes(2)
  expect(audios).toHaveLength(1)
  expect(audios[0]!.play).toHaveBeenCalledTimes(3)
  staleEnded()
  await tick()
  expect(state).toHaveBeenLastCalledWith("playing")
  audios[0]!.onended?.()
  await done
  expect(revoke).toHaveBeenCalledTimes(3)
  expect(state).toHaveBeenLastCalledWith("idle")
})
it("stops during a pending unlock and ignores its late resolution after replacement", async () => {
  let finishUnlock!: () => void
  const pendingUnlock = new Promise<void>((resolve) => {
    finishUnlock = resolve
  })
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      override play = vi.fn().mockImplementation(() => {
        this.paused = false
        return audios.length === 1 ? pendingUnlock : Promise.resolve()
      })
    },
  )
  const player = new SpeechPlayback(vi.fn())
  const firstSource = vi.fn(async function* () {
    yield new Blob()
  })
  const first = player.run(firstSource)
  player.stop()
  const second = player.run(async function* () {
    yield new Blob()
  })
  await tick()
  const replacement = audios[1]!
  const pauses = replacement.pause.mock.calls.length
  finishUnlock()
  await first
  expect(firstSource).not.toHaveBeenCalled()
  expect(replacement.pause).toHaveBeenCalledTimes(pauses)
  replacement.onended?.()
  await second
})
it("reports an actual voice autoplay denial without synthesis retries or fake playing", async () => {
  vi.stubGlobal(
    "Audio",
    class extends FakeAudio {
      override play = vi
        .fn()
        .mockResolvedValueOnce(undefined)
        .mockRejectedValue(
          new DOMException("Gesture required", "NotAllowedError"),
        )
    },
  )
  const state = vi.fn()
  const player = new SpeechPlayback(state)
  const synth = vi.fn().mockResolvedValue(new Blob())
  await player.run(async function* () {
    yield await synth()
  })
  expect(state).not.toHaveBeenCalledWith("playing")
  expect(state).toHaveBeenLastCalledWith("error")
  expect(synth).toHaveBeenCalledOnce()
  expect(revoke).toHaveBeenCalledTimes(2)
})

it("transfers exclusive ownership from article to preview without stale article cleanup stopping preview", async () => {
  const articleState = vi.fn()
  const previewState = vi.fn()
  const article = new SpeechPlayback(articleState)
  const preview = new SpeechPlayback(previewState)
  let articleSignal!: AbortSignal
  let finishArticleRequest!: (blob: Blob) => void
  const articleDone = article.run(async function* (signal) {
    articleSignal = signal
    yield new Blob()
    yield await new Promise<Blob>((resolve) => {
      finishArticleRequest = resolve
    })
  })
  await tick()
  const articleAudio = audios[0]!
  const staleEnded = articleAudio.onended!
  articleAudio.onended?.()
  await tick()
  const pausesBeforePreview = articleAudio.pause.mock.calls.length
  const previewDone = preview.run(async function* () {
    yield new Blob()
  })
  expect(articleSignal.aborted).toBe(true)
  expect(articleAudio.pause.mock.calls.length).toBeGreaterThan(
    pausesBeforePreview,
  )
  await tick()
  const previewAudio = audios[1]!
  const previewPauses = previewAudio.pause.mock.calls.length
  finishArticleRequest(new Blob())
  staleEnded()
  await articleDone
  article.stop()
  expect(previewAudio.pause).toHaveBeenCalledTimes(previewPauses)
  expect(previewState).toHaveBeenLastCalledWith("playing")
  previewAudio.onended?.()
  await previewDone
  expect(previewState).toHaveBeenLastCalledWith("idle")
  const previewUpdates = previewState.mock.calls.length
  const restartedArticle = article.run(async function* () {
    yield new Blob()
  })
  await tick()
  expect(previewState).toHaveBeenCalledTimes(previewUpdates)
  expect(articleState).toHaveBeenLastCalledWith("playing")
  audios[2]!.onended?.()
  await restartedArticle
  expect(articleState).toHaveBeenLastCalledWith("idle")
})
