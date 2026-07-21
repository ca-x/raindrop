import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import "../../reader/reader.css"
import type { EntryAiController } from "../model/useEntryAiController"
import { AiReaderSidecar } from "./AiReaderSidecar"

it("renders idle, processing, failed, and succeeded states without fake progress", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const enqueue = vi.fn().mockResolvedValue(true)
  const { rerender } = renderSidecar(controller({ enqueue }))

  await user.click(screen.getByRole("button", { name: "Generate summary" }))
  expect(enqueue).toHaveBeenCalledWith("summary")

  rerender(wrapped(controller({
    overview: overviewWithSummary("RUNNING"),
  })))
  const progress = screen.getByRole("progressbar", { name: "Processing" })
  expect(progress).not.toHaveAttribute("aria-valuenow")

  rerender(wrapped(controller({
    overview: overviewWithSummary("FAILED"),
  })))
  expect(screen.getByText("The Provider or plugin timed out.")).toBeVisible()

  rerender(wrapped(controller({
    overview: overviewWithSummary("SUCCEEDED"),
  })))
  expect(screen.getByText("A stable summary.")).toBeVisible()
})

it("offers AI settings for unavailable operations and closes explicitly", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onClose = vi.fn()
  const onOpenSettings = vi.fn()
  renderSidecar(
    controller({
      overview: {
        ...baseOverview(),
        availability: "NOT_CONFIGURED",
        summary: {
          operation: "SUMMARIZE",
          state: "UNAVAILABLE",
          job: null,
          artifact: null,
        },
      },
    }),
    { onClose, onOpenSettings },
  )

  await user.click(screen.getByRole("button", { name: "Open plugin settings" }))
  expect(onOpenSettings).toHaveBeenCalledOnce()
  await user.click(screen.getByRole("button", { name: "Close AI sidecar" }))
  expect(onClose).toHaveBeenCalledOnce()
})

it("keeps sidecar controls at the 44px touch-target minimum", () => {
  activateLocale("en")
  renderSidecar(controller())

  const close = screen.getByRole("button", { name: "Close AI sidecar" })
  const runSummary = screen.getByRole("button", { name: "Generate summary" })

  expect(getComputedStyle(close).minInlineSize).toBe("44px")
  expect(getComputedStyle(close).minBlockSize).toBe("44px")
  expect(getComputedStyle(runSummary).minBlockSize).toBe("44px")
})

function renderSidecar(
  entryController: EntryAiController,
  callbacks: { onClose?: () => void; onOpenSettings?: () => void } = {},
) {
  return render(
    wrapped(
      entryController,
      callbacks.onClose ?? vi.fn(),
      callbacks.onOpenSettings ?? vi.fn(),
    ),
  )
}

function wrapped(
  entryController: EntryAiController,
  onClose: () => void = vi.fn(),
  onOpenSettings: () => void = vi.fn(),
) {
  return (
    <Providers>
      <AiReaderSidecar
        controller={entryController}
        onClose={onClose}
        onOpenSettings={onOpenSettings}
      />
    </Providers>
  )
}

function controller(
  overrides: Partial<EntryAiController> = {},
): EntryAiController {
  return {
    entryId: "00000000-0000-4000-8000-000000000301",
    openTab: "summary",
    overview: baseOverview(),
    loadStatus: "ready",
    error: null,
    isMutating: false,
    open: vi.fn(),
    close: vi.fn(),
    enqueue: vi.fn().mockResolvedValue(true),
    retry: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    ...overrides,
  }
}

function baseOverview() {
  return {
    availability: "READY" as const,
    mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE" as const,
    summary: {
      operation: "SUMMARIZE" as const,
      state: "IDLE" as const,
      job: null,
      artifact: null,
    },
    translation: {
      operation: "TRANSLATE" as const,
      targetLocale: "zh-CN",
      state: "IDLE" as const,
      job: null,
      artifact: null,
    },
  }
}

function overviewWithSummary(state: "RUNNING" | "FAILED" | "SUCCEEDED") {
  const job = {
    jobId: "00000000-0000-4000-8000-000000000401",
    status: state,
    attempts: state === "RUNNING" ? 1 : 3,
    maxAttempts: 3 as const,
    nextAttemptAt: "2026-07-20T10:00:00Z",
    lastErrorCode: state === "FAILED" ? "PROVIDER_TIMEOUT" : null,
    createdAt: "2026-07-20T10:00:00Z",
    startedAt: "2026-07-20T10:00:01Z",
    completedAt: state === "RUNNING" ? null : "2026-07-20T10:00:03Z",
  }
  return {
    ...baseOverview(),
    summary: {
      operation: "SUMMARIZE" as const,
      state,
      job,
      artifact:
        state === "SUCCEEDED"
          ? {
              artifactId: "00000000-0000-4000-8000-000000000501",
              kind: "AI_SUMMARY" as const,
              providerLabel: "Primary model",
              createdAt: "2026-07-20T10:00:03Z",
              sourceLanguage: "en",
              summary: "A stable summary.",
              bullets: [],
              conclusion: null,
            }
          : null,
    },
  }
}
