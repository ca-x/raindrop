import { act, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { discoveredModelLimits, mergeModelLimits, recommendedModelBudget } from "../model/modelLimits"
import { ModelCapacityHint } from "./ModelCapacityHint"

afterEach(() => vi.unstubAllGlobals())
const limits = { contextWindow: 131072, maxInputTokens: 131072, maxOutputTokens: 32768, source: "COMBINED" as const }
const props = { csrfToken: "csrf", endpoint: "https://gateway.example/", model: "ollama-cloud/gpt-oss:120b", isDisabled: false, onApply: vi.fn() }

function view(model = props.model) {
  activateLocale("en")
  return <Providers><ModelCapacityHint {...props} model={model} /></Providers>
}

it("fetches metadata without a credential and applies balanced limits only on request", async () => {
  const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ catalogAvailable: true, limits })))
  vi.stubGlobal("fetch", fetchMock)
  props.onApply.mockReset()
  const user = userEvent.setup()
  render(view())
  await user.click(screen.getByRole("button", { name: "Look up model capacity" }))
  expect(await screen.findByText("32,768")).toBeVisible()
  expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ model: props.model, endpoint: props.endpoint })
  expect(props.onApply).not.toHaveBeenCalled()
  await user.click(screen.getByRole("button", { name: "Apply suggested limits" }))
  expect(props.onApply).toHaveBeenCalledWith({ maxInputTokensPerRequest: 98304, maxOutputTokensPerRequest: 32768 })
})

it("leaves manual setup available when metadata is unavailable", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ catalogAvailable: false, limits: null }))))
  const user = userEvent.setup()
  render(view())
  await user.click(screen.getByRole("button", { name: "Look up model capacity" }))
  expect(await screen.findByText("The model catalog is unavailable. You can set limits manually.")).toBeVisible()
  expect(screen.queryByRole("button", { name: "Apply suggested limits" })).not.toBeInTheDocument()
})

it("aborts and ignores a lookup for a previous model", async () => {
  let resolve!: (response: Response) => void
  const fetchMock = vi.fn(() => new Promise<Response>((done) => { resolve = done }))
  vi.stubGlobal("fetch", fetchMock)
  const user = userEvent.setup()
  const rendered = render(view())
  await user.click(screen.getByRole("button", { name: "Look up model capacity" }))
  const signal = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].signal!
  rendered.rerender(view("new-model"))
  expect(signal.aborted).toBe(true)
  await act(async () => resolve(new Response(JSON.stringify({ catalogAvailable: true, limits }))))
  await waitFor(() => expect(screen.queryByText("32,768")).not.toBeInTheDocument())
})

it("reserves context for input even when a model permits the whole context as output", () => {
  expect(recommendedModelBudget({ ...limits, maxOutputTokens: 131072 })).toEqual({ maxInputTokensPerRequest: 65536, maxOutputTokensPerRequest: 65536 })
  expect(recommendedModelBudget({ ...limits, contextWindow: null, maxInputTokens: null })).toBeNull()
  expect(mergeModelLimits({ contextWindow: null, maxInputTokens: null, maxOutputTokens: 8192, source: "PROVIDER" }, limits)).toEqual({ ...limits, maxOutputTokens: 8192 })
})

it("preserves known service caps across gateway aliases and keeps versions and variants distinct", () => {
  const models = [{ id: "deekseek/deepseek-41-flash", limits: { ...limits, maxOutputTokens: 8192 } }]
  expect(discoveredModelLimits(models, " auto/deepseek-4.1-flash ")?.maxOutputTokens).toBe(8192)
  expect(discoveredModelLimits(models, "auto/deepseek-4.2-flash")).toBeUndefined()
  expect(discoveredModelLimits(models, "auto/deepseek-4.1-flash-preview")).toBeUndefined()
  expect(discoveredModelLimits([...models, { id: "auto/deepseek-4.1-flash", limits }], "auto/deepseek-4.1-flash")?.maxOutputTokens).toBe(32768)
})
