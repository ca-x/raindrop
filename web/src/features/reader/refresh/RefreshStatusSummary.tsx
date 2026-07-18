import { Banner } from "@astryxdesign/core/Banner"
import { Stack } from "@astryxdesign/core/Stack"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

import type { Refresh } from "../api/subscription.generated"
import { formatRefreshTimestamp, refreshPresentation } from "./refreshPresentation"

interface RefreshStatusSummaryProps {
  refresh: Refresh | null
}

export function RefreshStatusSummary({ refresh }: RefreshStatusSummaryProps) {
  const { i18n } = useLingui()
  const status = refreshPresentation(refresh)
  const lastSuccess = timestampCopy(status.lastSuccessAt, i18n.locale)
  const lastSuccessCopy = lastSuccess
    ? i18n._("reader.refreshLastSuccess", { time: lastSuccess })
    : i18n._("reader.refreshNeverSucceeded")

  if (status.kind === "degraded") {
    return (
      <div className="reader-refresh-banner" aria-live="polite">
        <Banner
          container="section"
          status="warning"
          title={i18n._("reader.refreshDegradedTitle")}
          description={`${i18n._("reader.refreshDuplicateIssues", {
            count: status.duplicateCount,
          })} ${lastSuccessCopy}`}
        />
      </div>
    )
  }
  if (status.kind === "cooldown") {
    const retryAt = timestampCopy(status.retryAt, i18n.locale)
    const retryCopy = retryAt
      ? i18n._("reader.refreshRetryAt", { time: retryAt })
      : i18n._("reader.refreshRetryScheduled")
    return (
      <div className="reader-refresh-banner" aria-live="polite">
        <Banner
          container="section"
          status="warning"
          title={i18n._("reader.refreshCooldownTitle")}
          description={`${retryCopy} ${lastSuccessCopy}`}
        />
      </div>
    )
  }
  if (status.kind === "error") {
    return (
      <div className="reader-refresh-banner" aria-live="polite">
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.refreshError")}
          description={lastSuccessCopy}
        />
      </div>
    )
  }

  return (
    <Stack
      as="section"
      className="reader-refresh-summary"
      direction="horizontal"
      gap={2}
      align="center"
      aria-live="polite"
    >
      <StatusDot
        variant={status.tone}
        label={i18n._(status.label)}
        isPulsing={status.isPulsing}
      />
      <Stack className="reader-refresh-summary-copy" gap={0.5}>
        <Text type="label" display="block">{i18n._(status.label)}</Text>
        <Text type="supporting" display="block" textWrap="pretty">
          {summaryDetail(status.kind, lastSuccessCopy, (id) => i18n._(id))}
        </Text>
      </Stack>
    </Stack>
  )
}

function summaryDetail(
  kind: "idle" | "queued" | "running" | "ready",
  lastSuccessCopy: string,
  translate: (id: string) => string,
): string {
  switch (kind) {
    case "queued":
      return translate("reader.refreshQueuedSummary")
    case "running":
      return translate("reader.refreshRunningSummary")
    case "ready":
      return lastSuccessCopy
    case "idle":
      return translate("reader.refreshNeverSucceeded")
  }
}

function timestampCopy(value: string | null, locale: string): string | null {
  return formatRefreshTimestamp(value, locale || "en")
}
