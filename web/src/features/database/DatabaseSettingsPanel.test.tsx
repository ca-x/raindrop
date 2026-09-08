import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { DatabaseSettingsPanel } from "./DatabaseSettingsPanel"
import { databaseRequest, saveArticleRetention, type MaintenanceStatus } from "./api"
vi.mock("./api", () => ({ databaseRequest: vi.fn(), saveArticleRetention: vi.fn() }))
const storage = { databaseBytes: 100 * 1024 * 1024, walBytes: 0, freeBytes: 50 * 1024 * 1024,
  entryCount: 12060, feedCount: 41, tables: [{ name: "entries", bytes: 50 * 1024 * 1024 }] }
const initial: MaintenanceStatus = { running: false, storage, result: null, error: null,
  articleRetention: { enabled: false, retentionDays: 30 } }
function mount() {
  activateLocale("en")
  return render(<Providers><DatabaseSettingsPanel csrfToken="csrf-test" /></Providers>)
}
it("keeps cleanup disabled by default and confirms enabling before saving", async () => {
  vi.mocked(databaseRequest).mockResolvedValue(initial)
  vi.mocked(saveArticleRetention).mockResolvedValue({ enabled: true, retentionDays: 30 })
  const user = userEvent.setup()
  mount()
  expect(await screen.findByText("12,060")).toBeVisible()
  const toggle = screen.getByRole("switch", { name: "Automatic cleanup" })
  expect(toggle).not.toBeChecked()
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled()
  await user.click(toggle)
  await user.click(screen.getByRole("button", { name: "Save changes" }))
  expect(saveArticleRetention).not.toHaveBeenCalled()
  expect(screen.getByRole("alertdialog")).toBeVisible()
  await user.click(screen.getByRole("button", { name: "Confirm and save" }))
  await waitFor(() => expect(saveArticleRetention).toHaveBeenCalledWith({ enabled: true, retentionDays: 30 }, "csrf-test"))
  expect(await screen.findByText("Cleanup settings saved.")).toBeVisible()
})
it("starts one background compaction and displays the server's actual reclaimed size", async () => {
  vi.mocked(databaseRequest).mockResolvedValueOnce(initial).mockResolvedValueOnce({ ...initial, running: true })
    .mockResolvedValue({ ...initial, storage: { ...storage, databaseBytes: 50 * 1024 * 1024 }, result: {
      before: storage, after: { ...storage, databaseBytes: 50 * 1024 * 1024 }, reclaimedBytes: 50 * 1024 * 1024,
      walTruncated: true, removedRefreshRuns: 100,
    } })
  const user = userEvent.setup()
  mount()
  await user.click(await screen.findByRole("button", { name: "Compact database" }))
  expect(await screen.findByText("Freed 50 MiB. Total storage changed from 100 MiB to 50 MiB.")).toBeVisible()
  expect(vi.mocked(databaseRequest).mock.calls.filter(([csrf]) => csrf === "csrf-test")).toHaveLength(1)
})
it("shows a retry after status failure", async () => {
  vi.mocked(databaseRequest).mockRejectedValueOnce(new Error("offline")).mockResolvedValue(initial)
  mount()
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load database status")
  await userEvent.click(screen.getByRole("button", { name: "Retry" }))
  expect(await screen.findByText("12,060")).toBeVisible()
})
