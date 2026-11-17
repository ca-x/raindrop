import {
  apiRequest,
  apiResponse,
  invalidResponseError,
} from "../../shared/api/client"

export interface SpeechConfig {
  isEnabled: boolean
  baseUrl: string
  model: string
  voice: string
  speed: number
  hasApiKey: boolean
  keyringAvailable: boolean
  revision: number
}
export type SpeechDraft = Omit<
  SpeechConfig,
  "hasApiKey" | "keyringAvailable"
> & { apiKey?: string | null }
export interface SpeechPlan {
  entryId: string
  contentHash: string
  chunks: string[]
}
const ROOT = "/api/v1/plugins/tts"
export const speechEvents = new EventTarget()
export function stopAllSpeech() {
  speechEvents.dispatchEvent(new Event("stop"))
}
export function configChanged() {
  speechEvents.dispatchEvent(new Event("config"))
}
export async function getSpeechConfig(
  signal?: AbortSignal,
): Promise<SpeechConfig> {
  return readConfig(await apiRequest(ROOT, { signal }))
}
function readConfig(value: unknown): SpeechConfig {
  const config = value as SpeechConfig | undefined
  if (
    !config ||
    typeof config.isEnabled !== "boolean" ||
    typeof config.baseUrl !== "string" ||
    typeof config.model !== "string" ||
    typeof config.voice !== "string" ||
    typeof config.speed !== "number" ||
    !Number.isFinite(config.speed) ||
    config.speed < 0.25 ||
    config.speed > 4 ||
    typeof config.hasApiKey !== "boolean" ||
    typeof config.keyringAvailable !== "boolean" ||
    !Number.isSafeInteger(config.revision) ||
    config.revision < 0
  )
    throw invalidResponseError()
  return config
}
function mutation(
  csrf: string,
  body: unknown,
  signal?: AbortSignal,
): RequestInit {
  return {
    method: "POST",
    headers: { "x-csrf-token": csrf, "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  }
}
export async function putSpeechConfig(
  csrf: string,
  draft: SpeechDraft,
  signal?: AbortSignal,
) {
  return readConfig(
    await apiRequest(ROOT, { ...mutation(csrf, draft, signal), method: "PUT" }),
  )
}
async function audio(
  path: string,
  csrf: string,
  body: unknown,
  signal?: AbortSignal,
) {
  const response = await apiResponse(path, mutation(csrf, body, signal))
  if (
    response.headers.get("content-type")?.split(";")[0]?.trim() !== "audio/mpeg"
  )
    throw invalidResponseError()
  return response.blob()
}
export function testSpeech(
  csrf: string,
  draft: SpeechDraft,
  text: string,
  signal?: AbortSignal,
) {
  return audio(`${ROOT}/test`, csrf, { ...draft, text }, signal)
}
export async function planSpeech(
  id: string,
  csrf: string,
  signal?: AbortSignal,
): Promise<SpeechPlan> {
  const plan = (await apiRequest(
    `${ROOT}/entries/${encodeURIComponent(id)}/plan`,
    mutation(csrf, {}, signal),
  )) as SpeechPlan
  if (
    !plan ||
    plan.entryId !== id ||
    typeof plan.contentHash !== "string" ||
    !plan.contentHash ||
    !Array.isArray(plan.chunks) ||
    !plan.chunks.every((chunk) => typeof chunk === "string")
  )
    throw invalidResponseError()
  return plan
}
export function speakChunk(
  id: string,
  csrf: string,
  contentHash: string,
  chunkIndex: number,
  signal?: AbortSignal,
) {
  return audio(
    `${ROOT}/entries/${encodeURIComponent(id)}/speak`,
    csrf,
    { contentHash, chunkIndex },
    signal,
  )
}
