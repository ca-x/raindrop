import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Collapsible } from "@astryxdesign/core/Collapsible"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import {
  createProviderDraft,
  editProviderDraft,
  type ProviderDraft,
} from "../model/providerDraft"
import type { AiSettingsController } from "../model/useAiSettingsController"
import { ProviderForm } from "./ProviderForm"
import { ProviderList } from "./ProviderList"

export function ProviderSettingsPanel({
  controller,
}: {
  controller: AiSettingsController
}) {
  const { i18n } = useLingui()
  const [providerDraft, setProviderDraft] = useState<ProviderDraft | null>(null)
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return <Spinner label={i18n._("ai.settingsLoading")} />
  }
  if (controller.loadStatus === "error" || controller.keyringStatus === null) {
    return (
      <Banner
        status="error"
        title={i18n._("ai.settingsLoadError")}
        description={i18n._("ai.settingsLoadErrorDescription")}
      />
    )
  }
  return (
    <Stack gap={5} className="ai-settings-panel">
      {controller.keyringStatus === "UNAVAILABLE" ? (
        <Banner
          status="warning"
          title={i18n._("ai.keyringUnavailable")}
          description={i18n._("ai.keyringUnavailableDescription")}
        />
      ) : null}
      {controller.error ? (
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
              controller.clearError()
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
          onEdit={(provider) => {
            controller.clearError()
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
              draft={providerDraft}
              isSaving={controller.isSavingProvider}
              credentialAvailable={controller.keyringStatus === "AVAILABLE"}
              onChange={setProviderDraft}
              onSave={async (draft) => {
                const saved = await controller.saveProvider(draft)
                if (saved) setProviderDraft(null)
                return saved
              }}
              onCancel={() => setProviderDraft(null)}
            />
          </Collapsible>
        ) : null}
      </section>
    </Stack>
  )
}
