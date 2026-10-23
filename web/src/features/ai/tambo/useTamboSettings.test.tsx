import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import { ApiClientError } from "../../../shared/api/client"
import { loadTamboSettings, saveTamboSettings, type TamboSettings } from "./api"
import { useTamboSettings } from "./useTamboSettings"

vi.mock("./api", () => ({ loadTamboSettings: vi.fn(), saveTamboSettings: vi.fn() }))

const initial: TamboSettings = { configured: true, enabled: false, origin: "https://tambo.example", revision: null }

beforeEach(() => {
  vi.mocked(loadTamboSettings).mockReset().mockResolvedValue(initial)
  vi.mocked(saveTamboSettings).mockReset()
})

it("persists through the API and shares accepted revisions only with the same account", async () => {
  const storage = vi.spyOn(Storage.prototype, "setItem")
  const first = renderHook(() => useTamboSettings("user-a"))
  const second = renderHook(() => useTamboSettings("user-a"))
  const other = renderHook(() => useTamboSettings("user-b"))
  await waitFor(() => expect(first.result.current.settings).toEqual(initial))
  await waitFor(() => expect(second.result.current.settings).toEqual(initial))
  await waitFor(() => expect(other.result.current.settings).toEqual(initial))
  const saved = { ...initial, enabled: true, revision: 0 }
  vi.mocked(saveTamboSettings).mockResolvedValue(saved)
  await act(async () => { expect(await first.result.current.save(true, "csrf")).toBe(true) })
  expect(saveTamboSettings).toHaveBeenCalledWith(initial, true, "csrf")
  expect(second.result.current.settings).toEqual(saved)
  expect(other.result.current.settings).toEqual(initial)
  expect(storage).not.toHaveBeenCalled()
  first.unmount()
  vi.mocked(loadTamboSettings).mockResolvedValue(saved)
  const reopened = renderHook(() => useTamboSettings("user-a"))
  await waitFor(() => expect(reopened.result.current.settings).toEqual(saved))
})

it("ignores an old account's delayed response after switching accounts", async () => {
  let resolveOld!: (settings: TamboSettings) => void
  vi.mocked(loadTamboSettings).mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve }))
  const hook = renderHook(({ userId }) => useTamboSettings(userId), { initialProps: { userId: "user-a" } })
  hook.rerender({ userId: "user-b" })
  await waitFor(() => expect(hook.result.current.settings).toEqual(initial))
  await act(async () => { resolveOld({ ...initial, enabled: true, revision: 10 }) })
  expect(hook.result.current.settings).toEqual(initial)
})

it("reloads conflicts without automatically overwriting the newer preference", async () => {
  const hook = renderHook(() => useTamboSettings("user-a"))
  await waitFor(() => expect(hook.result.current.settings).toEqual(initial))
  vi.mocked(saveTamboSettings).mockRejectedValue(new ApiClientError(409, { code: "TAMBO_CONFLICT", message: "conflict" }))
  const latest = { ...initial, revision: 4 }
  vi.mocked(loadTamboSettings).mockResolvedValue(latest)
  await act(async () => { expect(await hook.result.current.save(true, "csrf")).toBe(false) })
  expect(hook.result.current.error).toBe("conflict")
  expect(hook.result.current.settings).toEqual(latest)
  expect(saveTamboSettings).toHaveBeenCalledOnce()
})

it("keeps the saved value when a write fails", async () => {
  const hook = renderHook(() => useTamboSettings("user-a"))
  await waitFor(() => expect(hook.result.current.settings).toEqual(initial))
  vi.mocked(saveTamboSettings).mockRejectedValue(new Error("offline"))
  await act(async () => { await hook.result.current.save(true, "csrf") })
  expect(hook.result.current.settings).toEqual(initial)
  expect(hook.result.current.error).toBe("save")
  expect(hook.result.current.isSaving).toBe(false)
})
