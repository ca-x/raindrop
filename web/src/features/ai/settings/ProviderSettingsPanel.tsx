import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Collapsible } from "@astryxdesign/core/Collapsible"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"

import {
  createProviderDraft,
  editProviderDraft,
  type ProviderDraft,
} from "../model/providerDraft"
import type { AiSettingsController } from "../model/useAiSettingsController"
import type { Provider } from "../api/provider.generated"
import { ProviderForm } from "./ProviderForm"
import { ProviderList } from "./ProviderList"

export function ProviderSettingsPanel({
  controller,
  onContinue,
}: {
  controller: AiSettingsController
  onContinue?: () => void
}) {
  const { i18n } = useLingui()
  const [providerDraft, setProviderDraft] = useState<ProviderDraft | null>(null)
  const [deleting, setDeleting] = useState<Provider | null>(null)
  const [notice, setNotice] = useState<"providerSaved" | "providerDeleted" | null>(null)
  const returnFocus = useRef<HTMLElement | null>(null)
  const editorWasOpen = useRef(false)
  const isEditorOpen = providerDraft !== null
  useEffect(() => {
    if (!isEditorOpen && editorWasOpen.current) returnFocus.current?.focus({ preventScroll: true })
    editorWasOpen.current = isEditorOpen
  }, [isEditorOpen])
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return <Spinner label={i18n._("ai.settingsLoading")} />
  }
  if (controller.loadStatus === "error" || controller.keyringStatus === null) {
    return (
      <Banner
        status="error"
        title={i18n._("ai.settingsLoadError")}
        description={i18n._("ai.settingsLoadErrorDescription")}
        endContent={<Button label={i18n._("common.retry")} variant="secondary" onClick={() => void controller.load()} />}
      />
    )
  }
  return (
    <Stack gap={5} className="ai-settings-panel">
      {notice ? (
        <div className="ai-provider-saved">
          <span role="status" className="reader-preference-description">{i18n._(`ai.${notice}`)}</span>
          {notice === "providerSaved" && onContinue ? (
            <Button label={i18n._("ai.providerContinueSetup")} variant="secondary" onClick={onContinue} />
          ) : null}
        </div>
      ) : null}
      {controller.keyringStatus === "UNAVAILABLE" ? (
        <Banner
          status="warning"
          title={i18n._("ai.keyringUnavailable")}
          description={i18n._("ai.keyringUnavailableDescription")}
        />
      ) : null}
      {controller.error && !providerDraft && !controller.error.startsWith("CONFIG") ? (
        <Banner
          status="error"
          title={i18n._("ai.settingsSaveError")}
          description={i18n._(`ai.error.${controller.error}`)}
        />
      ) : null}
      <section className="ai-settings-section" aria-labelledby="ai-providers-heading">
        <div className="ai-settings-section-heading">
          <div>
            <div id="ai-providers-heading" className="reader-preference-label">
              {i18n._("ai.providersTitle")}
            </div>
            <div className="reader-preference-description">
              {i18n._("ai.providersDescription")}
            </div>
          </div>
          <Button
            label={i18n._("ai.providerAdd")}
            onClick={() => {
              returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
              controller.clearError()
              setNotice(null)
              setProviderDraft(createProviderDraft())
            }}
            variant="secondary"
            isDisabled={
              providerDraft !== null ||
              controller.isSavingProvider ||
              controller.keyringStatus === "UNAVAILABLE"
            }
          />
        </div>
        <ProviderList
          providers={controller.providers}
          editingProviderId={providerDraft?.providerId ?? null}
          isBusy={controller.isSavingProvider || providerDraft !== null}
          onDelete={(provider) => { controller.clearError(); setDeleting(provider) }}
          onEdit={(provider) => {
            returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
            controller.clearError()
            setNotice(null)
            setProviderDraft(editProviderDraft(provider))
          }}
        />
        {providerDraft ? (
          <Collapsible
            trigger={i18n._(
              providerDraft.mode === "create"
                ? "ai.providerAddTitle"
                : "ai.providerEditTitle",
            )}
            isOpen
            onOpenChange={(isOpen) => {
              if (!isOpen && !controller.isSavingProvider) setProviderDraft(null)
            }}
            className="ai-provider-editor"
          >
            <ProviderForm
              key={providerDraft.providerId ?? "new"}
              csrfToken={controller.csrfToken}
              draft={providerDraft}
              isSaving={controller.isSavingProvider}
              saveError={controller.error ? i18n._(`ai.error.${controller.error}`) : null}
              credentialAvailable={controller.keyringStatus === "AVAILABLE"}
              onChange={(draft) => { controller.clearError(); setProviderDraft(draft) }}
              onSave={async (draft) => {
                const saved = await controller.saveProvider(draft)
                if (saved) {
                  setProviderDraft(null)
                  setNotice("providerSaved")
                }
                return saved
              }}
              onCancel={() => { setProviderDraft(null); controller.clearError() }}
              onDelete={providerDraft.providerId ? () => {
                const provider = controller.providers.find((item) => item.providerId === providerDraft.providerId)
                if (provider) { controller.clearError(); setDeleting(provider) }
              } : undefined}
            />
          </Collapsible>
        ) : null}
      </section>
      <AlertDialog
        isOpen={deleting !== null}
        onOpenChange={(open) => { if (!open && !controller.isSavingProvider) setDeleting(null) }}
        title={i18n._("ai.providerDeleteTitle")}
        description={i18n._("ai.providerDeleteDescription", { name: deleting?.displayName ?? "" })}
        actionLabel={i18n._("ai.providerDelete")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={controller.isSavingProvider}
        onAction={() => void (async () => {
          if (!deleting) return
          const removed = await controller.removeProvider(deleting)
          if (removed) {
            if (providerDraft?.providerId === deleting.providerId) setProviderDraft(null)
            setNotice("providerDeleted")
          }
          setDeleting(null)
        })()}
      />
    </Stack>
  )
}
