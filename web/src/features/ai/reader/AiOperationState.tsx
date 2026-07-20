import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { ProgressBar } from "@astryxdesign/core/ProgressBar"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

import type {
  AiAvailability,
  AiOperationOverview,
} from "../api/content.generated"
import type { EntryAiTab } from "../model/useEntryAiController"
import { SummaryView } from "./SummaryView"
import { TranslationView } from "./TranslationView"

interface AiOperationStateProps {
  tab: EntryAiTab
  availability: AiAvailability
  operation: AiOperationOverview
  isMutating: boolean
  onRun: () => Promise<boolean>
  onRetry: () => Promise<boolean>
  onOpenSettings?: () => void
}

export function AiOperationState(props: AiOperationStateProps) {
  const { i18n } = useLingui()
  const settingsAction = props.onOpenSettings ? (
    <Button
      label={i18n._("ai.reader.openPluginSettings")}
      onClick={props.onOpenSettings}
      variant="secondary"
    />
  ) : null

  if (props.operation.state === "UNAVAILABLE") {
    return (
      <StateStack>
        <Banner
          status="warning"
          title={i18n._("ai.reader.unavailable")}
          description={availabilityCopy(i18n._.bind(i18n), props.availability)}
        />
        {settingsAction}
      </StateStack>
    )
  }
  if (props.operation.state === "DISABLED") {
    return (
      <StateStack>
        <Banner
          status="info"
          title={i18n._("ai.reader.disabled")}
          description={i18n._("ai.reader.disabledDescription")}
        />
        {settingsAction}
      </StateStack>
    )
  }
  if (props.operation.state === "IDLE") {
    return (
      <StateStack>
        <Text as="p" display="block" color="secondary">
          {i18n._(
            props.tab === "summary"
              ? "ai.reader.summaryIdle"
              : "ai.reader.translationIdle",
          )}
        </Text>
        <Button
          label={i18n._(
            props.tab === "summary"
              ? "ai.reader.runSummary"
              : "ai.reader.runTranslation",
          )}
          clickAction={async () => {
            await props.onRun()
          }}
          variant="primary"
          isLoading={props.isMutating}
        />
      </StateStack>
    )
  }
  if (
    props.operation.state === "QUEUED" ||
    props.operation.state === "RUNNING" ||
    props.operation.state === "RETRY_WAIT"
  ) {
    return (
      <StateStack>
        <ProgressBar
          isIndeterminate
          label={i18n._(`ai.reader.state.${props.operation.state}`)}
        />
        <Text as="p" display="block" color="secondary">
          {i18n._("ai.reader.processingDescription")}
        </Text>
      </StateStack>
    )
  }
  if (props.operation.state === "FAILED") {
    return (
      <StateStack>
        <Banner
          status="error"
          title={i18n._("ai.reader.failed")}
          description={workerErrorCopy(
            i18n._.bind(i18n),
            props.operation.job?.lastErrorCode ?? null,
          )}
        />
        <Button
          label={i18n._("ai.reader.retry")}
          clickAction={async () => {
            await props.onRetry()
          }}
          variant="primary"
          isLoading={props.isMutating}
        />
      </StateStack>
    )
  }

  const artifact = props.operation.artifact
  if (props.tab === "summary" && artifact?.kind === "AI_SUMMARY") {
    return <SummaryView artifact={artifact} />
  }
  if (props.tab === "translation" && artifact?.kind === "AI_TRANSLATION") {
    return <TranslationView artifact={artifact} />
  }
  return (
    <Banner
      status="error"
      title={i18n._("ai.reader.resultInvalid")}
      description={i18n._("ai.reader.resultInvalidDescription")}
    />
  )
}

function StateStack({ children }: { children: React.ReactNode }) {
  return (
    <Stack gap={4} className="ai-operation-state" aria-live="polite">
      {children}
    </Stack>
  )
}

function availabilityCopy(
  translate: (id: string) => string,
  availability: AiAvailability,
): string {
  return translate(`ai.reader.availability.${availability}`)
}

function workerErrorCopy(
  translate: (id: string) => string,
  errorCode: string | null,
): string {
  if (errorCode?.includes("RATE_LIMITED")) {
    return translate("ai.reader.errorRateLimited")
  }
  if (errorCode?.includes("TIMEOUT")) return translate("ai.reader.errorTimeout")
  if (errorCode?.includes("STALE")) return translate("ai.reader.errorStale")
  if (errorCode?.includes("OUTPUT")) return translate("ai.reader.errorOutput")
  if (errorCode?.startsWith("PROVIDER_")) {
    return translate("ai.reader.errorProvider")
  }
  if (errorCode?.startsWith("PLUGIN_")) return translate("ai.reader.errorPlugin")
  return translate("ai.reader.errorGeneric")
}
