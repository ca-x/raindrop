import { Banner } from "@astryxdesign/core/Banner"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"

import type { AiSettingsController } from "../model/useAiSettingsController"
import { AiContentForm } from "./AiContentForm"
import { ProviderSettingsPanel } from "./ProviderSettingsPanel"

interface AiSettingsPanelProps {
  controller: AiSettingsController
}

export function AiSettingsPanel({ controller }: AiSettingsPanelProps) {
  const { i18n } = useLingui()

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

  const errorCopy = controller.error?.startsWith("CONFIG")
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
      {!pluginReady ? (
        <Banner
          status="warning"
          title={i18n._("ai.pluginUnavailable")}
          description={i18n._(
            `ai.pluginState.${controller.configEnvelope.pluginState}`,
          )}
        />
      ) : null}
      <ProviderSettingsPanel controller={controller} />
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
