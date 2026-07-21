import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { Tab, TabList } from "@astryxdesign/core/TabList"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState, type FormEvent } from "react"

import type { UserFont, UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"
import { AiSettingsPanel } from "../../ai/settings/AiSettingsPanel"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import {
  PersonalPreferencesForm,
  ReadingPreferencesForm,
} from "./AppearancePreferencesForm"

interface PreferencesDialogProps {
  isOpen: boolean
  initialTab?: PreferencesTab
  account?: { username: string; email: string | null }
  preferences: UserPreferences
  fonts: UserFont[]
  fontLimits: { maximumCount: number; maximumBytes: number }
  isSaving: boolean
  isFontMutating: boolean
  error: PreferencesControllerError | null
  aiController?: AiSettingsController
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onSave: (draft: UserPreferences) => Promise<boolean>
  onUploadFont: (file: File) => Promise<boolean>
  onDeleteFont: (fontId: string) => Promise<boolean>
}

export type PreferencesTab = "personal" | "reading" | "plugins"

export function PreferencesDialog(props: PreferencesDialogProps) {
  const { i18n } = useLingui()
  const [draft, setDraft] = useState(props.preferences)
  const [activeTab, setActiveTab] = useState<PreferencesTab>(
    props.initialTab ?? "personal",
  )
  const wasOpen = useRef(false)

  useEffect(() => {
    if (props.isOpen && !wasOpen.current) {
      setDraft(props.preferences)
      setActiveTab(props.initialTab ?? "personal")
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
  const deleteFont = async (fontId: string) => {
    const deleted = await props.onDeleteFont(fontId)
    if (deleted) {
      setDraft((current) =>
        current.readingCustomFontId === fontId
          ? { ...current, readingCustomFontId: null }
          : current,
      )
    }
    return deleted
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
            className="reader-dialog-header"
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
                <Tab value="personal" label={i18n._("preferences.tabPersonal")} />
                <Tab value="reading" label={i18n._("preferences.tabReading")} />
                {props.aiController ? (
                  <Tab value="plugins" label={i18n._("preferences.tabPlugins")} />
                ) : null}
              </TabList>
            </div>
            <div className="reader-preferences-panel">
              {activeTab === "personal" ? (
                <PersonalPreferencesForm
                  account={props.account ?? { username: "Raindrop", email: null }}
                  value={draft}
                  isSaving={props.isSaving}
                  error={props.error}
                  onChange={update}
                  onSubmit={submit}
                />
              ) : activeTab === "reading" ? (
                <ReadingPreferencesForm
                  value={draft}
                  isSaving={props.isSaving}
                  error={props.error}
                  fonts={props.fonts}
                  fontLimits={props.fontLimits}
                  isFontMutating={props.isFontMutating}
                  onUploadFont={props.onUploadFont}
                  onDeleteFont={deleteFont}
                  onChange={update}
                  onSubmit={submit}
                />
              ) : (
                <div role="tabpanel" aria-label={i18n._("preferences.tabPlugins")}>
                  {props.aiController ? <AiSettingsPanel controller={props.aiController} /> : null}
                </div>
              )}
            </div>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider padding={3}>
            <div className="reader-dialog-actions">
              {activeTab === "personal" || activeTab === "reading" ? (
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
