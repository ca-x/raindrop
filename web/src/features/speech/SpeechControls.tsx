import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"
import { getSpeechConfig, planSpeech, speakChunk, speechEvents } from "./api"
import { SpeechPlayback, type PlaybackState } from "./playback"
import "./speech.css"

export function SpeechControls({
  entryId,
  csrfToken,
}: {
  entryId: string
  csrfToken: string
}) {
  const { i18n } = useLingui()
  const translate = (key: string) => i18n._(`speech.${key}`)
  const [state, setState] = useState<PlaybackState>("idle")
  const [enabled, setEnabled] = useState(false)
  const [player] = useState(() => new SpeechPlayback(setState))
  useEffect(() => {
    let request: AbortController
    const load = () => {
      player.stop()
      setEnabled(false)
      request?.abort()
      request = new AbortController()
      const signal = request.signal
      void getSpeechConfig(signal)
        .then((config) => {
          if (!signal.aborted) setEnabled(config.isEnabled)
        })
        .catch(() => {
          if (!signal.aborted) setState("error")
        })
    }
    load()
    const stop = () => {
      request.abort()
      player.stop()
      setEnabled(false)
    }
    speechEvents.addEventListener("stop", stop)
    speechEvents.addEventListener("config", load)
    return () => {
      stop()
      speechEvents.removeEventListener("stop", stop)
      speechEvents.removeEventListener("config", load)
    }
  }, [entryId, csrfToken, player])
  if (!enabled) return null
  const action =
    state === "playing" ? "pause" : state === "paused" ? "resume" : "play"
  return (
    <div
      className="speech-controls"
      data-state={state}
      role="group"
      aria-label={translate("controls")}
    >
      <Button
        label={translate(action)}
        tooltip={translate(action)}
        icon={<Icon icon={state === "playing" ? PauseIcon : PlayIcon} />}
        isIconOnly
        variant="ghost"
        isLoading={state === "loading"}
        isDisabled={!csrfToken || state === "loading"}
        onClick={() => {
          if (state === "playing") player.pause()
          else if (state === "paused") void player.resume()
          else
            void player.run(async function* (signal) {
              const plan = await planSpeech(entryId, csrfToken, signal)
              for (let index = 0; index < plan.chunks.length; index++) {
                if (signal.aborted) return
                yield await speakChunk(
                  entryId,
                  csrfToken,
                  plan.contentHash,
                  index,
                  signal,
                )
              }
            })
        }}
      />
      <Button
        label={translate("stop")}
        tooltip={translate("stop")}
        icon={<Icon icon={StopIcon} />}
        isIconOnly
        variant="ghost"
        isDisabled={state === "idle" || state === "error"}
        onClick={() => player.stop()}
      />
      <span
        role={state === "error" ? "alert" : "status"}
        className={
          state === "idle" ? "reader-visually-hidden" : "speech-feedback"
        }
      >
        {translate(state === "error" ? "playbackError" : state)}
      </span>
    </div>
  )
}

function PlayIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinejoin="round"
    >
      <path d="m8 4 12 8-12 8Z" />
    </svg>
  )
}
function PauseIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
    >
      <path d="M8 4v16M16 4v16" />
    </svg>
  )
}
function StopIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
    >
      <rect x="5" y="5" width="14" height="14" rx="1" />
    </svg>
  )
}
