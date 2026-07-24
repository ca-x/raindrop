import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { BackupTarget } from "../api/backup.generated"
import type { BackupController } from "../model/useBackupController"
import { BackupSettingsPanel } from "./BackupSettingsPanel"

const targets: BackupTarget[] = [
  {
    targetId: "00000000-0000-4000-8000-000000000101",
    displayName: "Primary S3",
    enabled: true,
    config: {
      kind: "S3",
      settings: {
        endpoint: "https://objects.example/",
        region: "us-east-1",
        bucket: "reader-backups",
        prefix: "daily",
        pathStyle: true,
      },
    },
    retention: { retainCount: 7, retainDays: 30 },
    revision: 1,
    hasCredentials: true,
    createdAt: "2026-07-22T10:00:00Z",
    updatedAt: "2026-07-22T10:00:00Z",
  },
  {
    targetId: "00000000-0000-4000-8000-000000000102",
    displayName: "Archive S3",
    enabled: true,
    config: {
      kind: "S3",
      settings: {
        endpoint: "https://archive.example/",
        region: "auto",
        bucket: "rss-archive",
        prefix: "",
        pathStyle: false,
      },
    },
    retention: { retainCount: null, retainDays: 365 },
    revision: 1,
    hasCredentials: true,
    createdAt: "2026-07-22T10:00:00Z",
    updatedAt: "2026-07-22T10:00:00Z",
  },
  {
    targetId: "00000000-0000-4000-8000-000000000103",
    displayName: "Home WebDAV",
    enabled: true,
    config: {
      kind: "WEBDAV",
      settings: { endpoint: "https://dav.example/storage/", prefix: "rss" },
    },
    retention: { retainCount: 14, retainDays: null },
    revision: 1,
    hasCredentials: true,
    createdAt: "2026-07-22T10:00:00Z",
    updatedAt: "2026-07-22T10:00:00Z",
  },
]

it("lists multiple targets per provider and runs one job across selected S3 and WebDAV targets", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const runNow = vi.fn().mockResolvedValue(true)
  const controller = fakeController({ runNow })
  render(
    <Providers>
      <BackupSettingsPanel controller={controller} />
    </Providers>,
  )

  expect(screen.getByText("Primary S3")).toBeVisible()
  expect(screen.getByText("Archive S3")).toBeVisible()
  await user.click(screen.getByRole("button", { name: "Schedule" }))

  const selection = screen.getByLabelText("Backup targets")
  await user.click(within(selection).getByRole("checkbox", { name: /Primary S3/ }))
  await user.click(within(selection).getByRole("checkbox", { name: /Home WebDAV/ }))
  await user.click(screen.getByRole("button", { name: "Back up now" }))

  expect(runNow).toHaveBeenCalledWith([
    "00000000-0000-4000-8000-000000000101",
    "00000000-0000-4000-8000-000000000103",
  ])
})

it("uses aligned field slots for both S3 and WebDAV target forms", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  render(
    <Providers>
      <BackupSettingsPanel controller={fakeController({ targets: [] })} />
    </Providers>,
  )

  await user.click(screen.getByRole("button", { name: "Add S3 target" }))
  expect((await screen.findByLabelText(/^Name\b/)).closest(".reader-backup-form-field")).not.toBeNull()
  expect(screen.getByLabelText(/^HTTPS endpoint\b/).closest(".reader-backup-form-field")).not.toBeNull()
  const pathStyle = screen.getByRole("switch", { name: /Use path-style addressing/ })
  expect(pathStyle.closest(".astryx-switch-field")).toHaveAttribute("data-label-position", "start")
  expect(pathStyle.closest(".astryx-switch-field")).toHaveAttribute("data-label-spacing", "spread")

  await user.click(screen.getByRole("button", { name: "Cancel" }))
  await user.click(screen.getByRole("button", { name: "WebDAV" }))
  await user.click(screen.getByRole("button", { name: "Add WEBDAV target" }))
  expect((await screen.findByLabelText(/^Name\b/)).closest(".reader-backup-form-field")).not.toBeNull()
  expect(screen.getByLabelText(/^HTTPS endpoint\b/).closest(".reader-backup-form-field")).not.toBeNull()
})

function fakeController(
  overrides: Partial<BackupController> = {},
): BackupController {
  return {
    targets,
    schedule: {
      enabled: false,
      intervalHours: 24,
      targetIds: [],
      nextRunAt: null,
      revision: 0,
    },
    jobs: [],
    loadStatus: "ready",
    error: null,
    isMutating: false,
    activeTargetId: null,
    load: vi.fn().mockResolvedValue(undefined),
    refreshJobs: vi.fn().mockResolvedValue(undefined),
    createTarget: vi.fn().mockResolvedValue(true),
    updateTarget: vi.fn().mockResolvedValue(true),
    deleteTarget: vi.fn().mockResolvedValue(true),
    testTarget: vi.fn().mockResolvedValue(true),
    saveSchedule: vi.fn().mockResolvedValue(true),
    runNow: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    cancel: vi.fn(),
    ...overrides,
  }
}
