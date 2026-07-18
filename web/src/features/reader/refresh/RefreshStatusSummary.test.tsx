import { render, screen } from "@testing-library/react"
import { expect, it } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { Refresh } from "../api/subscription.generated"
import { RefreshStatusSummary } from "./RefreshStatusSummary"

it.each([
  ["QUEUED", "Queued for refresh", "Waiting for a refresh worker."],
  ["RUNNING", "Refreshing", "Fetching and processing feed updates."],
] as const)("renders a visible %s activity summary", (pendingState, title, detail) => {
  activateLocale("en")
  renderSummary(makeRefresh({ state: "PENDING", pendingState }))

  expect(screen.getByText(title)).toBeVisible()
  expect(screen.getByText(detail)).toBeVisible()
})

it("shows the last successful refresh for a ready feed", () => {
  activateLocale("en")
  renderSummary(makeRefresh())

  expect(screen.getByText("Refresh complete")).toBeVisible()
  expect(screen.getByText(/Last successful refresh:/)).toBeVisible()
})

it("shows bounded duplicate entry feedback in both locales", () => {
  activateLocale("en")
  const { unmount } = renderSummary(makeRefresh({
    state: "DEGRADED",
    droppedCount: 2,
    entryIssues: [{ code: "DUPLICATE_ENTRY", count: 2 }],
  }))
  expect(screen.getByRole("alert")).toHaveTextContent("2 duplicate entries were ignored.")
  unmount()

  activateLocale("zh-CN")
  renderSummary(makeRefresh({
    state: "DEGRADED",
    droppedCount: 2,
    entryIssues: [{ code: "DUPLICATE_ENTRY", count: 2 }],
  }))
  expect(screen.getByRole("alert")).toHaveTextContent("已忽略 2 个重复条目。")
})

it("shows cooldown retry timing and preserves the prior success", () => {
  activateLocale("en")
  renderSummary(makeRefresh({
    state: "BACKING_OFF",
    retryAt: "2026-07-18T03:00:00.000000Z",
  }))

  expect(screen.getByText("Refresh cooling down")).toBeVisible()
  expect(screen.getByText(/Next attempt after/)).toBeVisible()
  expect(screen.getByText(/Last successful refresh:/)).toBeVisible()
})

it("keeps the previous success visible after a refresh failure", () => {
  activateLocale("en")
  renderSummary(makeRefresh({ state: "ERROR" }))

  expect(screen.getByText("Refresh failed")).toBeVisible()
  expect(screen.getByText(/Last successful refresh:/)).toBeVisible()
})

function renderSummary(refresh: Refresh) {
  return render(
    <Providers>
      <RefreshStatusSummary refresh={refresh} />
    </Providers>,
  )
}

function makeRefresh(overrides: Partial<Refresh> = {}): Refresh {
  return {
    operationId: "00000000-0000-4000-8000-000000000801",
    state: "READY",
    pendingState: null,
    newCount: 1,
    updatedCount: 0,
    droppedCount: 0,
    entryIssues: [],
    generation: 2,
    errorCode: null,
    retryAt: null,
    lastSuccessAt: "2026-07-18T02:00:00.000000Z",
    queuedAt: "2026-07-18T01:59:58.000000Z",
    startedAt: "2026-07-18T01:59:59.000000Z",
    completedAt: "2026-07-18T02:00:00.000000Z",
    ...overrides,
  }
}
