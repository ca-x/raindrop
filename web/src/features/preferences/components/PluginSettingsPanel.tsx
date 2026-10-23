import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { List, ListItem } from "@astryxdesign/core/List"
import { StatusDot, type StatusDotVariant } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"

import { interactionMotion } from "../../../shared/motion/interactionMotion"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import { AiAssistantSettingsPanel } from "../../ai/settings/AiAssistantSettingsPanel"
import { ProviderSettingsPanel } from "../../ai/settings/ProviderSettingsPanel"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import { TranslationSettingsPanel } from "../../translation/settings/TranslationSettingsPanel"

interface PluginSettingsPanelProps {
  aiController: AiSettingsController
  translationController: TranslationSettingsController
  userId?: string
}

type PluginKey = "provider" | "assistant" | "translation"

export function PluginSettingsPanel({
  aiController,
  translationController,
  userId,
}: PluginSettingsPanelProps) {
  const { i18n } = useLingui()
  const [activePlugin, setActivePlugin] = useState<PluginKey | null>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    panelRef.current?.querySelector<HTMLElement>("[data-plugin-heading]")?.focus()
  }, [activePlugin])

  if (activePlugin) {
    const title = i18n._(`plugins.${activePlugin}.title`)
    return (
      <div ref={panelRef} className="reader-plugin-settings" {...interactionMotion}>
        <div className="reader-plugin-detail">
          <div className="reader-plugin-detail-heading">
            <Button
              label={i18n._("preferences.backToPlugins")}
              icon={<Icon icon="chevronLeft" />}
              onClick={() => setActivePlugin(null)}
              variant="ghost"
            />
            <div>
              <h3 className="reader-settings-title" tabIndex={-1} data-plugin-heading>{title}</h3>
              <div className="reader-preference-description">
                {i18n._(`plugins.${activePlugin}.detailDescription`)}
              </div>
            </div>
          </div>
          {activePlugin === "provider" ? (
            <ProviderSettingsPanel controller={aiController} onContinue={() => setActivePlugin(null)} />
          ) : activePlugin === "assistant" ? (
            <AiAssistantSettingsPanel controller={aiController} userId={userId} onOpenProviders={() => setActivePlugin("provider")} />
          ) : (
            <TranslationSettingsPanel
              controller={translationController}
              providers={aiController.providers}
            />
          )}
        </div>
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
  const hasProvider = providerStatus(aiController, (id) => i18n._(id)).variant === "success"
  const hasAssistant = assistantStatus(aiController, (id) => i18n._(id)).variant === "success"
  const hasTranslation = Boolean(translationController.config?.isEnabled)
  const nextPlugin = !hasProvider ? "provider" : !hasAssistant ? "assistant" : "translation"
  return (
    <div ref={panelRef} className="reader-plugin-settings" {...interactionMotion}>
      <section aria-labelledby="reader-plugins-heading" className="reader-plugin-index">
        <div>
          <h3 id="reader-plugins-heading" className="reader-settings-title" tabIndex={-1} data-plugin-heading>
            {i18n._("preferences.pluginsTitle")}
          </h3>
          <div className="reader-preference-description">
            {i18n._("preferences.pluginsDescription")}
          </div>
        </div>
        {aiController.loadStatus === "ready" && aiController.keyringStatus === "AVAILABLE" ? (
          <section className="reader-plugin-guide" aria-labelledby="reader-plugin-guide-heading">
            <div>
              <h3 id="reader-plugin-guide-heading">{i18n._("plugins.setupTitle")}</h3>
              <p className="reader-preference-description">{i18n._("plugins.setupDescription")}</p>
            </div>
            <ol className="reader-plugin-guide-steps">
              {[
                { label: "plugins.setupConnect", complete: hasProvider, optional: false },
                { label: "plugins.setupAssistant", complete: hasAssistant, optional: true },
                { label: "plugins.setupTranslation", complete: hasTranslation, optional: true },
              ].map((step, index) => (
                <li key={step.label} data-complete={step.complete}>
                  <span className="ai-setup-step-number" aria-hidden="true">{step.complete ? <Icon icon="check" /> : index + 1}</span>
                  <span>{i18n._(step.label)}</span>
                  <span className="reader-plugin-guide-meta">{i18n._(step.complete ? "plugins.setupComplete" : step.optional ? "plugins.setupOptional" : "preferences.pluginStatusNotConfigured")}</span>
                </li>
              ))}
            </ol>
            {!hasProvider || !hasAssistant || !hasTranslation ? (
              <Button label={i18n._(!hasProvider ? "plugins.setupStart" : !hasAssistant ? "plugins.setupNext" : "plugins.setupFinish")}
                onClick={() => setActivePlugin(nextPlugin)} variant="primary" />
            ) : null}
          </section>
        ) : null}
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
    </div>
  )
}

function providerStatus(
  controller: AiSettingsController,
  translate: (id: string) => string,
) {
  if (controller.loadStatus === "idle" || controller.loadStatus === "loading") {
    return status("neutral", translate("preferences.pluginStatusLoading"))
  }
  if (controller.loadStatus === "error" || controller.keyringStatus !== "AVAILABLE") {
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
  if (controller.configEnvelope.config?.isEnabled && controller.configEnvelope.config.summary.enabled) {
    const provider = controller.providers.find((item) => item.providerId === controller.configEnvelope?.config?.summary.providerId)
    return controller.keyringStatus === "AVAILABLE" && provider?.isEnabled
      ? status("success", translate("preferences.pluginStatusEnabled"))
      : status("warning", translate("preferences.pluginStatusUnavailable"))
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
