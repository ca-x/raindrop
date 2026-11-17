export type PlaybackState = "idle" | "loading" | "playing" | "paused" | "error"

export class SpeechPlayback {
  private static active?: SpeechPlayback
  private controller?: AbortController
  private audio?: HTMLAudioElement
  private url?: string
  private settle?: () => void
  private unpause?: () => void
  private paused = false
  private ended = false
  private playAttempt = 0
  private chunkRevision = 0
  private voiceReady = false
  constructor(private changed: (state: PlaybackState) => void) {}
  stop() {
    if (SpeechPlayback.active === this) SpeechPlayback.active = undefined
    this.controller?.abort()
    this.controller = undefined
    this.release()
    this.audio = undefined
    this.changed("idle")
  }
  private release() {
    this.playAttempt++
    this.chunkRevision++
    this.voiceReady = false
    this.paused = false
    this.ended = false
    if (this.audio) {
      this.audio.onended = null
      this.audio.onerror = null
      this.audio.pause()
      this.audio.removeAttribute("src")
      this.audio.load()
    }
    if (this.url) URL.revokeObjectURL(this.url)
    this.url = undefined
    this.settle?.()
    this.settle = undefined
    this.unpause?.()
    this.unpause = undefined
  }
  pause() {
    if (
      !this.voiceReady ||
      !this.audio ||
      !this.controller ||
      this.controller.signal.aborted
    )
      return
    this.playAttempt++
    this.paused = true
    this.audio.pause()
    this.changed("paused")
  }
  private async play(controller: AbortController, audio: HTMLAudioElement) {
    const attempt = ++this.playAttempt
    try {
      await audio.play()
    } catch (error) {
      if (
        !controller.signal.aborted &&
        this.audio === audio &&
        attempt === this.playAttempt &&
        !this.paused
      )
        throw error
      return
    }
    if (
      !controller.signal.aborted &&
      this.audio === audio &&
      attempt === this.playAttempt &&
      !this.paused &&
      !audio.paused &&
      !this.ended
    )
      this.changed("playing")
  }
  async resume() {
    const controller = this.controller
    const audio = this.audio
    if (!controller || controller.signal.aborted || !audio || !this.paused)
      return
    this.paused = false
    if (this.ended) {
      this.settle?.()
      this.unpause?.()
      this.unpause = undefined
      return
    }
    try {
      await this.play(controller, audio)
    } catch {
      if (!controller.signal.aborted) {
        this.stop()
        this.changed("error")
      }
    }
  }
  async run(source: (signal: AbortSignal) => AsyncIterable<Blob>) {
    if (SpeechPlayback.active && SpeechPlayback.active !== this)
      SpeechPlayback.active.stop()
    this.stop()
    SpeechPlayback.active = this
    const controller = new AbortController()
    this.controller = controller
    this.changed("loading")
    try {
      const audio = new Audio()
      this.audio = audio
      audio.muted = false
      this.url = URL.createObjectURL(silentWave())
      audio.src = this.url
      await new Promise<void>((resolve, reject) => {
        this.settle = resolve
        audio.onerror = () => reject(new Error("Audio unavailable"))
        void audio.play().then(resolve, reject)
      })
      if (controller.signal.aborted) return
      this.release()
      for await (const blob of source(controller.signal)) {
        if (controller.signal.aborted) break
        this.url = URL.createObjectURL(blob)
        audio.src = this.url
        this.voiceReady = true
        const chunkRevision = this.chunkRevision
        await new Promise<void>((resolve, reject) => {
          this.settle = resolve
          audio.onended = () => {
            if (
              controller.signal.aborted ||
              this.audio !== audio ||
              this.chunkRevision !== chunkRevision
            )
              return
            this.ended = true
            if (!this.paused) resolve()
          }
          audio.onerror = () => reject(new Error("Audio unavailable"))
          void this.play(controller, audio).catch(reject)
        })
        if (controller.signal.aborted) break
        if (this.paused)
          await new Promise<void>((resolve) => {
            this.unpause = resolve
          })
        if (controller.signal.aborted) break
        this.release()
        this.changed("loading")
      }
      if (!controller.signal.aborted) this.stop()
    } catch {
      if (!controller.signal.aborted) {
        this.stop()
        this.changed("error")
      }
    }
  }
}

function silentWave(): Blob {
  const samples = 160
  const buffer = new ArrayBuffer(44 + samples * 2)
  const view = new DataView(buffer)
  const writeTag = (offset: number, tag: string) => {
    for (let index = 0; index < tag.length; index++)
      view.setUint8(offset + index, tag.charCodeAt(index))
  }
  writeTag(0, "RIFF")
  view.setUint32(4, buffer.byteLength - 8, true)
  writeTag(8, "WAVE")
  writeTag(12, "fmt ")
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, 8000, true)
  view.setUint32(28, 16000, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  writeTag(36, "data")
  view.setUint32(40, samples * 2, true)
  return new Blob([buffer], { type: "audio/wav" })
}
