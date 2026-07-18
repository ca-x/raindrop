import type { Refresh } from "../api/subscription.generated"

export type RefreshPresentationKind =
  | "idle"
  | "queued"
  | "running"
  | "ready"
  | "degraded"
  | "cooldown"
  | "error"

export type RefreshPresentationTone = "neutral" | "success" | "warning" | "error"

export interface RefreshPresentation {
  kind: RefreshPresentationKind
  tone: RefreshPresentationTone
  label: string
  isPending: boolean
  isPulsing: boolean
  duplicateCount: number
  retryAt: string | null
  lastSuccessAt: string | null
  completedAt: string | null
}

export function refreshPresentation(refresh: Refresh | null): RefreshPresentation {
  const timing = {
    duplicateCount: duplicateIssueCount(refresh),
    retryAt: refresh?.retryAt ?? null,
    lastSuccessAt: refresh?.lastSuccessAt ?? null,
    completedAt: refresh?.completedAt ?? null,
  }
  if (!refresh) return presentation("idle", "neutral", "reader.refreshIdle", false, false, timing)
  if (refresh.pendingState === "RUNNING") {
    return presentation("running", "warning", "reader.refreshRunning", true, true, timing)
  }
  if (refresh.pendingState === "QUEUED" || refresh.state === "PENDING") {
    return presentation("queued", "warning", "reader.refreshQueued", true, true, timing)
  }
  switch (refresh.state) {
    case "READY":
      return presentation("ready", "success", "reader.refreshReady", false, false, timing)
    case "DEGRADED":
      return presentation("degraded", "warning", "reader.refreshDegraded", false, false, timing)
    case "BACKING_OFF":
      return presentation("cooldown", "warning", "reader.refreshCooldown", false, false, timing)
    case "ERROR":
      return presentation("error", "error", "reader.refreshError", false, false, timing)
    default:
      return presentation("idle", "neutral", "reader.refreshIdle", false, false, timing)
  }
}

export function formatRefreshTimestamp(value: string | null, locale: string): string | null {
  if (!value) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date)
}

function duplicateIssueCount(refresh: Refresh | null): number {
  return refresh?.entryIssues
    .filter((issue) => issue.code === "DUPLICATE_ENTRY")
    .reduce((total, issue) => total + issue.count, 0) ?? 0
}

function presentation(
  kind: RefreshPresentationKind,
  tone: RefreshPresentationTone,
  label: string,
  isPending: boolean,
  isPulsing: boolean,
  timing: Pick<
    RefreshPresentation,
    "duplicateCount" | "retryAt" | "lastSuccessAt" | "completedAt"
  >,
): RefreshPresentation {
  return { kind, tone, label, isPending, isPulsing, ...timing }
}
