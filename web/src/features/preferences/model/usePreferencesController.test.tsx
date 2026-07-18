import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"
import type { ReactNode } from "react"

import { Providers } from "../../../app/Providers"
import { ApiClientError } from "../../../shared/api/client"
import { activateLocale } from "../../../shared/i18n/i18n"
import type {
  PatchUserPreferencesRequest,
  UserPreferences,
} from "../api/preferences.generated"
import { PREFERENCE_HINT_KEY } from "./preferenceCache"
import {
  usePreferencesController,
  type PreferencesApi,
} from "./usePreferencesController"

const serverPreferences: UserPreferences = {
  locale: "en",
  themeMode: "LIGHT",
  layoutDensity: "SPACIOUS",
  readingFontScale: 110,
}

beforeEach(() => {
  localStorage.clear()
  activateLocale("en")
})

it("loads server preferences without blocking the initialized runtime", async () => {
  const pending = deferred<UserPreferences>()
  const api = fakeApi({ getPreferences: vi.fn(() => pending.promise) })
  const { result } = renderController(api)

  let load: Promise<void>
  act(() => {
    load = result.current.load()
  })
  expect(result.current.preferences).toEqual({
    locale: "en",
    themeMode: "SYSTEM",
    layoutDensity: "BALANCED",
    readingFontScale: 100,
  })
  expect(result.current.loadStatus).toBe("loading")

  pending.resolve(serverPreferences)
  await act(async () => load)
  expect(result.current.loadStatus).toBe("ready")
  expect(result.current.preferences).toEqual(serverPreferences)
})

it("keeps the runtime usable and exposes a stable load error", async () => {
  const api = fakeApi({
    getPreferences: vi.fn().mockRejectedValue(new Error("network details")),
  })
  const { result } = renderController(api)

  await act(async () => result.current.load())

  expect(result.current.loadStatus).toBe("error")
  expect(result.current.error).toBe("LOAD")
  expect(result.current.preferences.themeMode).toBe("SYSTEM")
})

it("saves only changed fields and applies the authoritative response", async () => {
  const api = fakeApi()
  const { result } = renderController(api)
  await act(async () => result.current.load())
  const persisted = { ...serverPreferences, themeMode: "DARK" as const }
  vi.mocked(api.patchPreferences).mockResolvedValueOnce(persisted)

  await act(async () => {
    await expect(
      result.current.save({
        ...serverPreferences,
        themeMode: "DARK",
      }),
    ).resolves.toBe(true)
  })

  expect(api.patchPreferences).toHaveBeenCalledWith(
    "csrf-memory",
    { themeMode: "DARK" },
    expect.any(AbortSignal),
  )
  expect(result.current.preferences).toEqual(persisted)
  expect(result.current.error).toBeNull()
})

it("returns early when the draft has not changed", async () => {
  const api = fakeApi()
  const { result } = renderController(api)
  await act(async () => result.current.load())

  await act(async () => {
    await expect(result.current.save(serverPreferences)).resolves.toBe(true)
  })
  expect(api.patchPreferences).not.toHaveBeenCalled()
})

it("rolls back an optimistic runtime update when saving fails", async () => {
  const pending = deferred<UserPreferences>()
  const api = fakeApi({ patchPreferences: vi.fn(() => pending.promise) })
  const { result } = renderController(api)
  await act(async () => result.current.load())
  const draft = { ...serverPreferences, readingFontScale: 120 }

  let save: Promise<boolean>
  act(() => {
    save = result.current.save(draft)
  })
  await waitFor(() => expect(result.current.preferences).toEqual(draft))
  expect(result.current.isSaving).toBe(true)

  pending.reject(new Error("redacted network failure"))
  await act(async () => expect(save).resolves.toBe(false))
  expect(result.current.preferences).toEqual(serverPreferences)
  expect(result.current.error).toBe("SAVE")
})

it("clears the hint and ends the session after an authentication rejection", async () => {
  localStorage.setItem(PREFERENCE_HINT_KEY, "persisted-presentation-only")
  const onUnauthenticated = vi.fn()
  const api = fakeApi({
    getPreferences: vi.fn().mockRejectedValue(
      new ApiClientError(401, {
        code: "AUTHENTICATION_REQUIRED",
        message: "Authentication is required",
      }),
    ),
  })
  const { result } = renderController(api, onUnauthenticated)

  await act(async () => result.current.load())

  expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
})

function renderController(api: PreferencesApi, onUnauthenticated = vi.fn()) {
  return renderHook(
    () =>
      usePreferencesController({
        csrfToken: "csrf-memory",
        onUnauthenticated,
        api,
      }),
    { wrapper: RuntimeWrapper },
  )
}

function RuntimeWrapper({ children }: { children: ReactNode }) {
  return <Providers>{children}</Providers>
}

function fakeApi(overrides: Partial<PreferencesApi> = {}): PreferencesApi {
  return {
    getPreferences: vi.fn().mockResolvedValue(serverPreferences),
    patchPreferences: vi.fn<
      (
        csrfToken: string,
        patch: PatchUserPreferencesRequest,
        signal?: AbortSignal,
      ) => Promise<UserPreferences>
    >(),
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
