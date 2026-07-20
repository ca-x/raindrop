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
import { AiContentForm } from "./AiContentForm"
import { ProviderForm } from "./ProviderForm"
import { ProviderList } from "./ProviderList"

interface AiSettingsPanelProps {
  controller: AiSettingsController
}

export function AiSettingsPanel({ controller }: AiSettingsPanelProps) {
  const { i18n } = useLingui()
  const [providerDraft, setProviderDraft] = useState<ProviderDraft | null>(null)

  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return (
      <div className="ai-settings-loading">
        <Spinner label={i18n._("ai.settingsLoading")} />
      </div>
    )
  }
  if (
    controller.loadStatus === "error" ||
    controller.configEnvelope === null ||
    controller.keyringStatus === null
  ) {
    return (
      <Banner
        status="error"
        title={i18n._("ai.settingsLoadError")}
        description={i18n._("ai.settingsLoadErrorDescription")}
      />
    )
  }

  const errorCopy = controller.error
    ? aiErrorCopy((id) => i18n._(id), controller.error)
    : null
  const pluginReady = controller.configEnvelope.pluginState === "READY"
  return (
    <Stack gap={6} className="ai-settings-panel">
      {errorCopy ? (
        <Banner
          status="error"
          title={errorCopy.title}
          description={errorCopy.description}
        />
      ) : null}
      {controller.keyringStatus === "UNAVAILABLE" ? (
        <Banner
          status="warning"
          title={i18n._("ai.keyringUnavailable")}
          description={i18n._("ai.keyringUnavailableDescription")}
        />
      ) : null}
      {!pluginReady ? (
        <Banner
          status="warning"
          title={i18n._("ai.pluginUnavailable")}
          description={i18n._(
            `ai.pluginState.${controller.configEnvelope.pluginState}`,
          )}
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
            tooltip={
              controller.keyringStatus === "UNAVAILABLE"
                ? i18n._("ai.providerCredentialUnavailable")
                : undefined
            }
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
      <section className="ai-settings-section" aria-labelledby="ai-content-heading">
        <div>
          <div id="ai-content-heading" className="reader-preference-label">
            {i18n._("ai.contentTitle")}
          </div>
          <div className="reader-preference-description">
            {i18n._("ai.contentDescription")}
          </div>
        </div>
        <AiContentForm
          providers={controller.providers}
          envelope={controller.configEnvelope}
          isSaving={controller.isSavingConfig}
          onSave={controller.saveConfig}
        />
      </section>
    </Stack>
  )
}

function aiErrorCopy(
  translate: (id: string) => string,
  error: NonNullable<AiSettingsController["error"]>,
) {
  const conflict = error === "PROVIDER_CONFLICT" || error === "CONFIG_CONFLICT"
  return {
    title: translate(conflict ? "ai.revisionConflict" : "ai.settingsSaveError"),
    description: translate(`ai.error.${error}`),
  }
}
