import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { Tab, TabList } from "@astryxdesign/core/TabList"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState, type FormEvent } from "react"

import type { UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"
import { AiSettingsPanel } from "../../ai/settings/AiSettingsPanel"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import { OpmlTransferPanel } from "../../opml/components/OpmlTransferPanel"
import { AppearancePreferencesForm } from "./AppearancePreferencesForm"

interface PreferencesDialogProps {
  isOpen: boolean
  initialTab?: PreferencesTab
  preferences: UserPreferences
  isSaving: boolean
  error: PreferencesControllerError | null
  aiController?: AiSettingsController
  csrfToken: string
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onSave: (draft: UserPreferences) => Promise<boolean>
  onSubscriptionsChanged: () => Promise<void> | void
}

export type PreferencesTab = "appearance" | "ai" | "subscriptions"

export function PreferencesDialog(props: PreferencesDialogProps) {
  const { i18n } = useLingui()
  const [draft, setDraft] = useState(props.preferences)
  const [activeTab, setActiveTab] = useState<PreferencesTab>(
    props.initialTab ?? "appearance",
  )
  const wasOpen = useRef(false)

  useEffect(() => {
    if (props.isOpen && !wasOpen.current) {
      setDraft(props.preferences)
      setActiveTab(props.initialTab ?? "appearance")
    }
    wasOpen.current = props.isOpen
  }, [props.initialTab, props.isOpen, props.preferences])

  const close = () => {
    if (props.isSaving) return
    props.onClearError()
    props.onOpenChange(false)
  }
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (props.isSaving) return
    if (await props.onSave(draft)) props.onOpenChange(false)
  }
  const update = (patch: Partial<UserPreferences>) => {
    setDraft((current) => ({ ...current, ...patch }))
    props.onClearError()
  }
  return (
    <Dialog
      isOpen={props.isOpen}
      aria-label={i18n._("preferences.title")}
      onOpenChange={(open) => {
        if (!open) close()
      }}
      purpose="form"
      width="min(600px, calc(100vw - 24px))"
      maxHeight="min(720px, calc(100dvh - 24px))"
      className="reader-preferences-dialog"
    >
      <Layout
        height="fill"
        padding={0}
        header={
          <DialogHeader
            title={i18n._("preferences.title")}
            subtitle={i18n._("preferences.description")}
            hasDivider
          />
        }
        content={
          <LayoutContent padding={0} className="reader-preferences-content">
            <div className="reader-preferences-tabs">
              <TabList
                value={activeTab}
                onChange={(value) => setActiveTab(value as PreferencesTab)}
                layout="fill"
                hasDivider
              >
                <Tab value="appearance" label={i18n._("preferences.tabAppearance")} />
                {props.aiController ? (
                  <Tab value="ai" label={i18n._("ai.settingsTab")} />
                ) : null}
                <Tab
                  value="subscriptions"
                  label={i18n._("preferences.tabSubscriptions")}
                />
              </TabList>
            </div>
            <div className="reader-preferences-panel">
              {activeTab === "appearance" ? (
                <AppearancePreferencesForm
                  value={draft}
                  isSaving={props.isSaving}
                  error={props.error}
                  onChange={update}
                  onSubmit={submit}
                />
              ) : activeTab === "ai" && props.aiController ? (
                <div role="tabpanel" aria-label={i18n._("ai.settingsTab")}>
                  <AiSettingsPanel controller={props.aiController} />
                </div>
              ) : (
                <div role="tabpanel" aria-label={i18n._("preferences.tabSubscriptions")}>
                  <OpmlTransferPanel
                    csrfToken={props.csrfToken}
                    onImported={props.onSubscriptionsChanged}
                  />
                </div>
              )}
            </div>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider padding={3}>
            <div className="reader-dialog-actions">
              {activeTab === "appearance" ? (
                <>
                  <Button
                    label={i18n._("common.cancel")}
                    onClick={close}
                    isDisabled={props.isSaving}
                    variant="secondary"
                  />
                  <Button
                    label={i18n._("preferences.save")}
                    type="submit"
                    form="reader-preferences-form"
                    isLoading={props.isSaving}
                    variant="primary"
                  />
                </>
              ) : (
                <Button
                  label={i18n._("common.close")}
                  onClick={close}
                  variant="secondary"
                />
              )}
            </div>
          </LayoutFooter>
        }
      />
    </Dialog>
  )
}
