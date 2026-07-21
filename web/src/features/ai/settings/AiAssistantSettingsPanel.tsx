import { Banner } from "@astryxdesign/core/Banner"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"

import type { AiSettingsController } from "../model/useAiSettingsController"
import { AiContentForm } from "./AiContentForm"

export function AiAssistantSettingsPanel({
  controller,
}: {
  controller: AiSettingsController
}) {
  const { i18n } = useLingui()
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return <Spinner label={i18n._("ai.settingsLoading")} />
  }
  if (controller.loadStatus === "error" || !controller.configEnvelope) {
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
      {controller.error ? (
        <Banner
          status="error"
          title={i18n._("ai.settingsSaveError")}
          description={i18n._(`ai.error.${controller.error}`)}
        />
      ) : null}
      {controller.configEnvelope.pluginState !== "READY" ? (
        <Banner
          status="warning"
          title={i18n._("ai.pluginUnavailable")}
          description={i18n._(
            `ai.pluginState.${controller.configEnvelope.pluginState}`,
          )}
        />
      ) : null}
      <AiContentForm
        providers={controller.providers}
        envelope={controller.configEnvelope}
        isSaving={controller.isSavingConfig}
        onSave={controller.saveConfig}
      />
    </Stack>
  )
}
