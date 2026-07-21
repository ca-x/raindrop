import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Heading } from "@astryxdesign/core/Heading"
import { Icon } from "@astryxdesign/core/Icon"
import { Spinner } from "@astryxdesign/core/Spinner"
import { useLingui } from "@lingui/react"
import { useEffect, useRef } from "react"

import type { EntryAiController } from "../model/useEntryAiController"
import { AiOperationState } from "./AiOperationState"

interface AiReaderSidecarProps {
  controller: EntryAiController
  onClose: () => void
  onOpenSettings?: () => void
}

export function AiReaderSidecar(props: AiReaderSidecarProps) {
  const { i18n } = useLingui()
  const headingRef = useRef<HTMLHeadingElement>(null)
  const tab = props.controller.openTab
  useEffect(() => {
    headingRef.current?.focus({ preventScroll: true })
  }, [props.controller.entryId])
  if (!tab) return null

  const overview = props.controller.overview
  const operation = overview?.summary ?? null
  return (
    <section
      className="reader-ai-sidecar"
      aria-labelledby="reader-ai-sidecar-heading"
    >
      <div className="reader-ai-sidecar-header">
        <Heading
          ref={headingRef}
          id="reader-ai-sidecar-heading"
          level={2}
          tabIndex={-1}
        >
          {i18n._("ai.reader.title")}
        </Heading>
        <Button
          label={i18n._("ai.reader.close")}
          icon={<Icon icon="close" />}
          isIconOnly
          tooltip={i18n._("ai.reader.close")}
          onClick={props.onClose}
          variant="ghost"
        />
      </div>
      <div className="reader-ai-sidecar-content" aria-live="polite">
        {props.controller.error ? (
          <Banner
            status="error"
            title={i18n._("ai.reader.requestError")}
            description={i18n._(`ai.reader.controllerError.${props.controller.error}`)}
          />
        ) : null}
        {props.controller.loadStatus === "error" ? (
          <Banner
            status="error"
            title={i18n._("ai.reader.loadError")}
            description={i18n._("ai.reader.loadErrorDescription")}
          />
        ) : props.controller.loadStatus === "loading" || !operation || !overview ? (
          <div className="reader-ai-sidecar-loading">
            <Spinner label={i18n._("ai.reader.loading")} />
          </div>
        ) : (
          <AiOperationState
            tab="summary"
            availability={overview.availability}
            operation={operation}
            isMutating={props.controller.isMutating}
            onRun={() => props.controller.enqueue("summary")}
            onRetry={() => props.controller.retry("summary")}
            onOpenSettings={props.onOpenSettings}
          />
        )}
      </div>
    </section>
  )
}
