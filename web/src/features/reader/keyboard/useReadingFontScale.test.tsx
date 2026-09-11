import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { defaultPreferences } from "../../preferences/model/preferenceTypes"
import { usePreferencesController, type PreferencesApi } from "../../preferences/model/usePreferencesController"
import { useReadingFontScale } from "./useReadingFontScale"

beforeEach(() => {
  localStorage.clear()
  activateLocale("en")
})

it("updates repeated presses immediately and coalesces writes while a request is pending", async () => {
  const { result, release, api } = renderScale()
  act(() => result.current.scale.change(1))
  expect(result.current.scale.scale).toBe(105)
  act(() => {
    result.current.scale.change(1)
    result.current.scale.change(1)
  })
  expect(result.current.scale.scale).toBe(115)
  expect(api.patchPreferences).toHaveBeenCalledTimes(1)
  await act(async () => release())
  await waitFor(() => expect(result.current.scale.isPending).toBe(false))
  expect(result.current.preferences.readingFontScale).toBe(115)
  expect(api.patchPreferences).toHaveBeenCalledTimes(2)
})

it("honors reset pressed during an outstanding resize", async () => {
  const { result, release } = renderScale()
  act(() => result.current.scale.change(1))
  act(() => result.current.scale.change(0))
  expect(result.current.scale.scale).toBe(100)
  await act(async () => release())
  await waitFor(() => expect(result.current.scale.isPending).toBe(false))
  expect(result.current.preferences.readingFontScale).toBe(100)
})

it("rolls back failed resize batches and accepts a subsequent retry", async () => {
  const { result, release } = renderScale(true)
  act(() => result.current.scale.change(1))
  act(() => result.current.scale.change(1))
  expect(result.current.scale.scale).toBe(110)
  await act(async () => release())
  await waitFor(() => expect(result.current.scale.isPending).toBe(false))
  expect(result.current.scale.scale).toBe(100)
  expect(result.current.error).toBe("SAVE")
  act(() => result.current.scale.change(-1))
  await waitFor(() => expect(result.current.scale.isPending).toBe(false))
  expect(result.current.preferences.readingFontScale).toBe(95)
})

function renderScale(failFirst = false) {
  let persisted = defaultPreferences("en")
  let release!: () => void
  const pending = new Promise<void>((resolve) => { release = resolve })
  let first = true
  const api: PreferencesApi = {
    getPreferences: vi.fn(async () => persisted),
    patchPreferences: vi.fn(async (_csrf, patch) => {
      if (first) {
        first = false
        await pending
        if (failFirst) throw new Error("offline")
      }
      persisted = { ...persisted, ...patch }
      return persisted
    }),
    listUserFonts: vi.fn(),
    uploadUserFont: vi.fn(),
    deleteUserFont: vi.fn(),
  }
  const { result } = renderHook(() => {
    const controller = usePreferencesController({ csrfToken: "csrf", api, onUnauthenticated: vi.fn() })
    return { ...controller, scale: useReadingFontScale(controller) }
  }, { wrapper: Providers })
  return { result, release, api }
}
