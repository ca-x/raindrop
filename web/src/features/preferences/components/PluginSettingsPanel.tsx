import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { List, ListItem } from "@astryxdesign/core/List"
import { StatusDot, type StatusDotVariant } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import { AiAssistantSettingsPanel } from "../../ai/settings/AiAssistantSettingsPanel"
import { ProviderSettingsPanel } from "../../ai/settings/ProviderSettingsPanel"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import { TranslationSettingsPanel } from "../../translation/settings/TranslationSettingsPanel"

interface PluginSettingsPanelProps {
  aiController: AiSettingsController
  translationController: TranslationSettingsController
}

type PluginKey = "provider" | "assistant" | "translation"

export function PluginSettingsPanel({
  aiController,
  translationController,
}: PluginSettingsPanelProps) {
  const { i18n } = useLingui()
  const [activePlugin, setActivePlugin] = useState<PluginKey | null>(null)

  if (activePlugin) {
    const title = i18n._(`plugins.${activePlugin}.title`)
    return (
      <div className="reader-plugin-detail">
        <div className="reader-plugin-detail-heading">
          <Button
            label={i18n._("preferences.backToPlugins")}
            icon={<Icon icon="chevronLeft" />}
            onClick={() => setActivePlugin(null)}
            variant="ghost"
          />
          <div>
            <div className="reader-settings-title">{title}</div>
            <div className="reader-preference-description">
              {i18n._(`plugins.${activePlugin}.detailDescription`)}
            </div>
          </div>
        </div>
        {activePlugin === "provider" ? (
          <ProviderSettingsPanel controller={aiController} />
        ) : activePlugin === "assistant" ? (
          <AiAssistantSettingsPanel controller={aiController} />
        ) : (
          <TranslationSettingsPanel
            controller={translationController}
            providers={aiController.providers}
          />
        )}
      </div>
    )
  }

  const plugins: Array<{
    key: PluginKey
    status: { variant: StatusDotVariant; label: string }
  }> = [
    { key: "provider", status: providerStatus(aiController, (id) => i18n._(id)) },
    { key: "assistant", status: assistantStatus(aiController, (id) => i18n._(id)) },
    {
      key: "translation",
      status: translationStatus(translationController, (id) => i18n._(id)),
    },
  ]
  return (
    <section aria-labelledby="reader-plugins-heading" className="reader-plugin-index">
      <div>
        <div id="reader-plugins-heading" className="reader-settings-title">
          {i18n._("preferences.pluginsTitle")}
        </div>
        <div className="reader-preference-description">
          {i18n._("preferences.pluginsDescription")}
        </div>
      </div>
      <List density="balanced" hasDividers className="reader-plugin-list">
        {plugins.map(({ key, status }) => {
          const title = i18n._(`plugins.${key}.title`)
          return (
            <ListItem
              key={key}
              label={title}
              description={i18n._(`plugins.${key}.description`)}
              startContent={
                <StatusDot
                  variant={status.variant}
                  label={status.label}
                  tooltip={status.label}
                />
              }
              endContent={
                <div className="reader-plugin-list-end">
                  <span className="reader-plugin-status">{status.label}</span>
                  <Button
                    aria-label={i18n._("preferences.pluginSettingsFor", {
                      plugin: title,
                    })}
                    label={i18n._("preferences.pluginSettings")}
                    onClick={() => setActivePlugin(key)}
                    variant="secondary"
                  />
                </div>
              }
            />
          )
        })}
      </List>
    </section>
  )
}

function providerStatus(
  controller: AiSettingsController,
  translate: (id: string) => string,
) {
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return status("neutral", translate("preferences.pluginStatusLoading"))
  }
  if (controller.loadStatus === "error" || controller.keyringStatus === null) {
    return status("error", translate("preferences.pluginStatusUnavailable"))
  }
  if (controller.providers.some((provider) => provider.isEnabled)) {
    return status("success", translate("preferences.pluginStatusEnabled"))
  }
  if (controller.providers.length > 0) {
    return status("neutral", translate("preferences.pluginStatusDisabled"))
  }
  return status("neutral", translate("preferences.pluginStatusNotConfigured"))
}

function assistantStatus(
  controller: AiSettingsController,
  translate: (id: string) => string,
) {
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return status("neutral", translate("preferences.pluginStatusLoading"))
  }
  if (controller.loadStatus === "error" || !controller.configEnvelope) {
    return status("error", translate("preferences.pluginStatusUnavailable"))
  }
  if (controller.configEnvelope.pluginState !== "READY") {
    return status("warning", translate("preferences.pluginStatusUnavailable"))
  }
  if (controller.configEnvelope.config?.isEnabled) {
    return status("success", translate("preferences.pluginStatusEnabled"))
  }
  if (controller.configEnvelope.config) {
    return status("neutral", translate("preferences.pluginStatusDisabled"))
  }
  return status("neutral", translate("preferences.pluginStatusNotConfigured"))
}

function translationStatus(
  controller: TranslationSettingsController,
  translate: (id: string) => string,
) {
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return status("neutral", translate("preferences.pluginStatusLoading"))
  }
  if (controller.loadStatus === "error" || !controller.config) {
    return status("error", translate("preferences.pluginStatusUnavailable"))
  }
  if (controller.config.isEnabled) {
    return status("success", translate("preferences.pluginStatusEnabled"))
  }
  if (controller.config.revision !== null) {
    return status("neutral", translate("preferences.pluginStatusDisabled"))
  }
  return status("neutral", translate("preferences.pluginStatusNotConfigured"))
}

function status(variant: StatusDotVariant, label: string) {
  return { variant, label }
}
