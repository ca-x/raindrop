import { expect, it } from "vitest"

import type { Refresh } from "../api/subscription.generated"
import { formatRefreshTimestamp, refreshPresentation } from "./refreshPresentation"

it.each([
  [null, "idle", false, "reader.refreshIdle"],
  [makeRefresh({ state: "PENDING", pendingState: "QUEUED" }), "queued", true, "reader.refreshQueued"],
  [makeRefresh({ state: "PENDING", pendingState: "RUNNING" }), "running", true, "reader.refreshRunning"],
  [makeRefresh({ state: "READY" }), "ready", false, "reader.refreshReady"],
  [makeRefresh({ state: "DEGRADED", droppedCount: 2, entryIssues: [{ code: "DUPLICATE_ENTRY", count: 2 }] }), "degraded", false, "reader.refreshDegraded"],
  [makeRefresh({ state: "BACKING_OFF" }), "cooldown", false, "reader.refreshCooldown"],
  [makeRefresh({ state: "ERROR" }), "error", false, "reader.refreshError"],
] as const)("maps refresh state to %s presentation", (refresh, kind, isPending, label) => {
  expect(refreshPresentation(refresh)).toMatchObject({ kind, isPending, label })
})

it("projects bounded duplicate issue counts and timing inputs", () => {
  const presentation = refreshPresentation(makeRefresh({
    state: "DEGRADED",
    droppedCount: 5,
    entryIssues: [{ code: "DUPLICATE_ENTRY", count: 5 }],
    retryAt: "2026-07-18T03:00:00.000000Z",
    lastSuccessAt: "2026-07-18T02:00:00.000000Z",
  }))

  expect(presentation).toMatchObject({
    duplicateCount: 5,
    retryAt: "2026-07-18T03:00:00.000000Z",
    lastSuccessAt: "2026-07-18T02:00:00.000000Z",
  })
})

it("formats valid timestamps and rejects absent or invalid values", () => {
  expect(formatRefreshTimestamp(null, "en")).toBeNull()
  expect(formatRefreshTimestamp("invalid", "en")).toBeNull()
  expect(formatRefreshTimestamp("2026-07-18T02:00:00.000000Z", "en")).toContain("2026")
  expect(formatRefreshTimestamp("2026-07-18T02:00:00.000000Z", "zh-CN")).toContain("2026")
})

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
