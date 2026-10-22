import { Icon } from "@astryxdesign/core/Icon"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"

import type { Refresh } from "../api/subscription.generated"
import { formatRefreshTimestamp, refreshPresentation } from "./refreshPresentation"

interface RefreshStatusSummaryProps {
  refresh: Refresh | null
}

export function RefreshStatusSummary({ refresh }: RefreshStatusSummaryProps) {
  const { i18n } = useLingui()
  const status = refreshPresentation(refresh)
  const label = i18n._(status.kind === "cooldown" ? "reader.refreshCooldownTitle"
    : status.kind === "degraded" ? "reader.refreshDegradedTitle" : status.label)
  const lastSuccess = formatRefreshTimestamp(status.lastSuccessAt, i18n.locale || "en")
  const lastSuccessCopy = lastSuccess
    ? i18n._("reader.refreshLastSuccess", { time: lastSuccess })
    : i18n._("reader.refreshNeverSucceeded")
  const retryAt = formatRefreshTimestamp(status.retryAt, i18n.locale || "en")
  const detail = status.kind === "cooldown"
    ? retryAt ? i18n._("reader.refreshRetryAt", { time: retryAt }) : i18n._("reader.refreshRetryScheduled")
    : status.kind === "degraded"
      ? i18n._("reader.refreshDuplicateIssues", { count: status.duplicateCount })
      : status.kind === "queued" ? i18n._("reader.refreshQueuedSummary")
        : status.kind === "running" ? i18n._("reader.refreshRunningSummary") : null

  return (
    <details className="reader-refresh-ready" data-tone={status.tone}>
      <summary>
        <StatusDot variant={status.isPending ? "neutral" : status.tone}
          label={label} isPulsing={status.isPulsing} />
        <span role="status">{label}</span>
        {lastSuccess && status.lastSuccessAt ? (
          <time dateTime={status.lastSuccessAt} title={lastSuccessCopy}>
            {new Intl.DateTimeFormat(i18n.locale || "en", {
              hour: "2-digit", minute: "2-digit",
            }).format(new Date(status.lastSuccessAt))}
          </time>
        ) : null}
        <Icon icon="chevronDown" />
      </summary>
      {detail ? <p>{detail}</p> : null}
      <p>{lastSuccessCopy}</p>
      {status.kind !== "ready" && status.kind !== "idle" ? (
        <p>{i18n._("reader.refreshBackgroundHint")}</p>
      ) : null}
    </details>
  )
}
