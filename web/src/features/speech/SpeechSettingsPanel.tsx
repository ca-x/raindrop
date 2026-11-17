import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import { Selector } from "@astryxdesign/core/Selector"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { Switch } from "@astryxdesign/core/Switch"
import { TextInput } from "@astryxdesign/core/TextInput"
import { TextArea } from "@astryxdesign/core/TextArea"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"
import {
  configChanged,
  getSpeechConfig,
  putSpeechConfig,
  speechEvents,
  testSpeech,
  type SpeechConfig,
  type SpeechDraft,
} from "./api"
import { SpeechPlayback, type PlaybackState } from "./playback"

export const VOICES = [
  ["zh-CN-YunyangNeural", "云扬 · zh-CN-YunyangNeural"],
  ["zh-CN-XiaoxiaoNeural", "晓晓 · zh-CN-XiaoxiaoNeural"],
  ["zh-CN-YunxiNeural", "云希 · zh-CN-YunxiNeural"],
  ["en-US-JennyNeural", "Jenny · en-US-JennyNeural"],
  ...["alloy", "echo", "fable", "onyx", "nova", "shimmer", "coral"].map(
    (voiceId) => [voiceId, `OpenAI · ${voiceId}`],
  ),
]
export function SpeechSettingsPanel({
  csrfToken = "",
}: {
  csrfToken?: string
}) {
  const { i18n } = useLingui()
  const translate = (key: string) => i18n._(`speech.${key}`)
  const [draft, setDraft] = useState<SpeechDraft>()
  const [config, setConfig] = useState<SpeechConfig>()
  const [custom, setCustom] = useState(false)
  const [text, setText] = useState("")
  const [error, setError] = useState(false)
  const [saved, setSaved] = useState(false)
  const [saving, setSaving] = useState(false)
  const [state, setState] = useState<PlaybackState>("idle")
  const [player] = useState(() => new SpeechPlayback(setState))
  const requests = useRef(new AbortController())
  useEffect(() => {
    requests.current = new AbortController()
    const controller = new AbortController()
    void getSpeechConfig(controller.signal)
      .then((config) => {
        if (controller.signal.aborted) return
        setConfig(config)
        setDraft(toDraft(config))
        setCustom(!VOICES.some(([voiceId]) => voiceId === config.voice))
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true)
      })
    const stop = () => {
      controller.abort()
      requests.current.abort()
      player.stop()
    }
    speechEvents.addEventListener("stop", stop)
    return () => {
      stop()
      speechEvents.removeEventListener("stop", stop)
    }
  }, [player])
  const update = (patch: Partial<SpeechDraft>) => {
    setDraft((currentDraft) => currentDraft && { ...currentDraft, ...patch })
    setError(false)
    setSaved(false)
    player.stop()
  }
  const valid = () => {
    if (!draft) return false
    try {
      const url = new URL(draft.baseUrl)
      return (
        url.protocol === "https:" &&
        !url.username &&
        !url.password &&
        !url.pathname.replace(/\/+$/u, "").endsWith("/audio/speech") &&
        !!draft.model.trim() &&
        !!draft.voice.trim() &&
        Number.isFinite(draft.speed) &&
        draft.speed >= 0.25 &&
        draft.speed <= 4
      )
    } catch {
      return false
    }
  }
  if (!draft || !config)
    return error ? (
      <Banner status="error" title={translate("error")} />
    ) : (
      <Spinner label={translate("loading")} />
    )
  const busy =
    saving || state === "loading" || state === "playing" || state === "paused"
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault()
        if (!valid()) {
          setError(true)
          return
        }
        setSaving(true)
        setError(false)
        setSaved(false)
        void putSpeechConfig(csrfToken, draft, requests.current.signal)
          .then((config) => {
            if (requests.current.signal.aborted) return
            setConfig(config)
            setDraft(toDraft(config))
            setSaved(true)
            configChanged()
          })
          .catch(() => {
            if (!requests.current.signal.aborted) setError(true)
          })
          .finally(() => {
            if (!requests.current.signal.aborted) setSaving(false)
          })
      }}
    >
      <Stack gap={5}>
        {error || state === "error" ? (
          <Banner status="error" title={translate("error")} />
        ) : null}
        {saved ? <Banner status="success" title={translate("saved")} /> : null}
        <Switch
          label={translate("enabled")}
          value={draft.isEnabled}
          onChange={(isEnabled) => update({ isEnabled })}
          isDisabled={busy}
        />
        <TextInput
          label={translate("baseUrl")}
          description={translate("baseUrlHelp")}
          value={draft.baseUrl}
          onChange={(baseUrl) => update({ baseUrl })}
          isDisabled={busy}
          width="100%"
        />
        <TextInput
          label={translate("apiKey")}
          type="password"
          description={translate(config.hasApiKey ? "keySaved" : "keyOptional")}
          value={draft.apiKey ?? ""}
          onChange={(apiKey) => update({ apiKey: apiKey || undefined })}
          isDisabled={busy || !config.keyringAvailable}
          width="100%"
        />
        {!config.keyringAvailable ? (
          <p className="reader-preference-description">
            {translate("keyringUnavailable")}
          </p>
        ) : null}
        {config.hasApiKey ? (
          <Button
            label={translate(draft.apiKey === null ? "keepKey" : "clearKey")}
            variant="secondary"
            isDisabled={busy}
            onClick={() =>
              update({ apiKey: draft.apiKey === null ? undefined : null })
            }
          />
        ) : null}
        {draft.apiKey === null ? (
          <p role="status">{translate("keyCleared")}</p>
        ) : null}
        <TextInput
          label={translate("model")}
          value={draft.model}
          onChange={(model) => update({ model })}
          isDisabled={busy}
          width="100%"
        />
        <Selector
          label={translate("voice")}
          description={translate("voiceHelp")}
          value={custom ? "CUSTOM" : draft.voice}
          options={[
            ...VOICES.map(([value, label]) => ({
              value: value!,
              label: label!,
            })),
            { value: "CUSTOM", label: translate("custom") },
          ]}
          onChange={(voice) => {
            setCustom(voice === "CUSTOM")
            if (voice !== "CUSTOM") update({ voice })
          }}
          isDisabled={busy}
          width="100%"
        />
        {custom ? (
          <TextInput
            label={translate("customVoice")}
            value={draft.voice}
            onChange={(voice) => update({ voice })}
            isDisabled={busy}
            width="100%"
          />
        ) : null}
        <NumberInput
          label={translate("speed")}
          value={draft.speed}
          min={0.25}
          max={4}
          step={0.25}
          onChange={(speed) => update({ speed })}
          isDisabled={busy}
          width="100%"
        />
        <TextArea
          label={translate("previewText")}
          description={translate("cost")}
          value={text}
          onChange={setText}
          isDisabled={busy}
          rows={3}
          width="100%"
        />
        {state === "loading" || state === "playing" ? (
          <p role="status" className="reader-preference-description">
            {translate(state)}
          </p>
        ) : null}
        <div className="reader-dialog-actions">
          <Button
            label={translate("test")}
            variant="secondary"
            isDisabled={busy || !csrfToken}
            isLoading={state === "loading"}
            onClick={() => {
              if (!valid() || !text.trim() || Array.from(text).length > 200) {
                setError(true)
                return
              }
              setError(false)
              void player.run(async function* (signal) {
                yield await testSpeech(csrfToken, draft, text, signal)
              })
            }}
          />
          {busy && !saving ? (
            <Button
              label={translate("stop")}
              variant="ghost"
              onClick={() => player.stop()}
            />
          ) : null}
          <Button
            label={translate("save")}
            type="submit"
            isLoading={saving}
            isDisabled={busy || !csrfToken}
            variant="primary"
          />
        </div>
      </Stack>
    </form>
  )
}
function toDraft(config: SpeechConfig): SpeechDraft {
  return {
    isEnabled: config.isEnabled,
    baseUrl: config.baseUrl,
    model: config.model,
    voice: config.voice,
    speed: config.speed,
    revision: config.revision,
  }
}
