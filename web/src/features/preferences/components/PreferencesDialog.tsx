import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { Icon } from "@astryxdesign/core/Icon"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react"

import type { UserFont, UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import type { UserProfile } from "../../profile/api/profile.generated"
import type {
  ProfileControllerError,
  ProfileFieldError,
} from "../../profile/model/useProfileController"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import type { BackupController } from "../../backups/model/useBackupController"
import { BackupSettingsPanel } from "../../backups/components/BackupSettingsPanel"
import {
  PersonalPreferencesForm,
  ReadingPreferencesForm,
} from "./AppearancePreferencesForm"
import { PluginSettingsPanel } from "./PluginSettingsPanel"

interface PreferencesDialogProps {
  isOpen: boolean
  initialTab?: PreferencesTab
  profile: UserProfile
  preferences: UserPreferences
  fonts: UserFont[]
  fontLimits: { maximumCount: number; maximumBytes: number }
  isSaving: boolean
  isProfileSaving: boolean
  isFontMutating: boolean
  error: PreferencesControllerError | null
  profileError: ProfileControllerError | null
  profileFieldErrors: Partial<Record<"displayName" | "email", ProfileFieldError>>
  aiController?: AiSettingsController
  translationController?: TranslationSettingsController
  backupController?: BackupController
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onClearProfileError: () => void
  onSave: (draft: UserPreferences) => Promise<boolean>
  onSaveProfile: (draft: UserProfile) => Promise<boolean>
  onUploadFont: (file: File) => Promise<boolean>
  onDeleteFont: (fontId: string) => Promise<boolean>
}

export type PreferencesTab = "personal" | "reading" | "plugins" | "backup" | "about"

export function PreferencesDialog(props: PreferencesDialogProps) {
  const { i18n } = useLingui()
  const [draft, setDraft] = useState(props.preferences)
  const [profileDraft, setProfileDraft] = useState(props.profile)
  const [localProfileFieldErrors, setLocalProfileFieldErrors] = useState<
    PreferencesDialogProps["profileFieldErrors"]
  >({})
  const [activeTab, setActiveTab] = useState<PreferencesTab>(
    props.initialTab ?? "personal",
  )
  const wasOpen = useRef(false)

  useEffect(() => {
    if (props.isOpen && !wasOpen.current) {
      setDraft(props.preferences)
      setProfileDraft(props.profile)
      setLocalProfileFieldErrors({})
      setActiveTab(props.initialTab ?? "personal")
    }
    wasOpen.current = props.isOpen
  }, [props.initialTab, props.isOpen, props.preferences, props.profile])

  const close = () => {
    if (props.isSaving || props.isProfileSaving) return
    props.onClearError()
    props.onClearProfileError()
    props.onOpenChange(false)
  }
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (props.isSaving || props.isProfileSaving) return
    if (activeTab === "personal") {
      const normalized = normalizedProfile(profileDraft)
      const validation = validateProfile(normalized)
      setLocalProfileFieldErrors(validation)
      if (Object.keys(validation).length > 0) return
      if (!(await props.onSaveProfile(normalized))) return
    }
    if (await props.onSave(draft)) props.onOpenChange(false)
  }
  const update = (patch: Partial<UserPreferences>) => {
    setDraft((current) => ({ ...current, ...patch }))
    props.onClearError()
  }
  const updateProfile = (profile: UserProfile) => {
    setProfileDraft(profile)
    setLocalProfileFieldErrors({})
    props.onClearProfileError()
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
      width="min(960px, calc(100vw - 24px))"
      maxHeight="min(780px, calc(100dvh - 24px))"
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
          <LayoutContent padding={0} isScrollable={false} className="reader-preferences-content">
            <div className="reader-settings-layout">
              <nav className="reader-settings-nav" aria-label={i18n._("preferences.sections")}>
                <SettingsNavButton
                  isActive={activeTab === "personal"}
                  label={i18n._("preferences.tabPersonal")}
                  description={i18n._("preferences.tabPersonalDescription")}
                  icon={<PersonalIcon />}
                  onClick={() => setActiveTab("personal")}
                />
                <SettingsNavButton
                  isActive={activeTab === "reading"}
                  label={i18n._("preferences.tabReading")}
                  description={i18n._("preferences.tabReadingDescription")}
                  icon={<ReadingIcon />}
                  onClick={() => setActiveTab("reading")}
                />
                {props.aiController && props.translationController ? (
                  <SettingsNavButton
                    isActive={activeTab === "plugins"}
                    label={i18n._("preferences.tabPlugins")}
                    description={i18n._("preferences.tabPluginsDescription")}
                    icon={<Icon icon="wrench" size="sm" color="inherit" />}
                    onClick={() => setActiveTab("plugins")}
                  />
                ) : null}
                {props.backupController ? (
                  <SettingsNavButton
                    isActive={activeTab === "backup"}
                    label={i18n._("preferences.tabBackup")}
                    description={i18n._("preferences.tabBackupDescription")}
                    icon={<BackupIcon />}
                    onClick={() => setActiveTab("backup")}
                  />
                ) : null}
                <SettingsNavButton
                  isActive={activeTab === "about"}
                  label={i18n._("preferences.tabAbout")}
                  description={i18n._("preferences.tabAboutDescription")}
                  icon={<Icon icon="info" size="sm" color="inherit" />}
                  onClick={() => setActiveTab("about")}
                />
              </nav>
              <div
                key={activeTab}
                className="reader-preferences-panel reader-panel-transition"
              >
                {activeTab === "personal" || activeTab === "reading" ? (
                  <div className="reader-settings-panel-intro">
                    <div className="reader-settings-title">
                      {i18n._(
                        activeTab === "personal"
                          ? "preferences.personalTitle"
                          : "preferences.readingTitle",
                      )}
                    </div>
                    <div className="reader-preference-description">
                      {i18n._(
                        activeTab === "personal"
                          ? "preferences.personalDescription"
                          : "preferences.readingDescription",
                      )}
                    </div>
                  </div>
                ) : null}
                {activeTab === "personal" ? (
                  <PersonalPreferencesForm
                    profile={profileDraft}
                    profileError={props.profileError}
                    profileFieldErrors={{
                      ...props.profileFieldErrors,
                      ...localProfileFieldErrors,
                    }}
                    onProfileChange={updateProfile}
                    value={draft}
                    isSaving={props.isSaving || props.isProfileSaving}
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
                ) : activeTab === "plugins" && props.aiController && props.translationController ? (
                  <PluginSettingsPanel
                    aiController={props.aiController}
                    translationController={props.translationController}
                  />
                ) : activeTab === "backup" && props.backupController ? (
                  <BackupSettingsPanel controller={props.backupController} />
                ) : activeTab === "about" ? (
                  <AboutPanel />
                ) : null}
              </div>
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
                    isDisabled={props.isSaving || props.isProfileSaving}
                    variant="secondary"
                  />
                  <Button
                    label={i18n._("preferences.save")}
                    type="submit"
                    form="reader-preferences-form"
                    isLoading={props.isSaving || props.isProfileSaving}
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

function SettingsNavButton(props: {
  isActive: boolean
  label: string
  description: string
  icon: ReactNode
  onClick: () => void
}) {
  return (
    <button
      type="button"
      className="reader-settings-nav-item"
      aria-current={props.isActive ? "page" : undefined}
      onClick={props.onClick}
    >
      <span className="reader-settings-nav-icon" aria-hidden="true">{props.icon}</span>
      <span className="reader-settings-nav-copy">
        <span className="reader-settings-nav-label">{props.label}</span>
        <span className="reader-settings-nav-description">{props.description}</span>
      </span>
    </button>
  )
}

function AboutPanel() {
  const { i18n } = useLingui()
  return (
    <section className="reader-about-panel">
      <div className="reader-about-mark" aria-hidden="true">R</div>
      <div>
        <div className="reader-settings-title">Raindrop</div>
        <div className="reader-about-version">v{__RAINDROP_VERSION__}</div>
      </div>
      <p className="reader-preference-description">{i18n._("preferences.aboutDescription")}</p>
    </section>
  )
}

function PersonalIcon() {
  return <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><circle cx="10" cy="6" r="3" /><path d="M4 17c.5-3.2 2.5-5 6-5s5.5 1.8 6 5" strokeLinecap="round" /></svg>
}

function ReadingIcon() {
  return <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M3 4.5c2.8-.7 5.1-.2 7 1.4v10c-1.9-1.6-4.2-2.1-7-1.4v-10ZM17 4.5c-2.8-.7-5.1-.2-7 1.4v10c1.9-1.6 4.2-2.1 7-1.4v-10Z" strokeLinejoin="round" /></svg>
}

function BackupIcon() {
  return <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M5.5 15.5h9a3 3 0 0 0 .4-6 5 5 0 0 0-9.6-1.4A3.8 3.8 0 0 0 5.5 15.5Z" strokeLinejoin="round" /><path d="M10 8.5v5M8 10.5l2-2 2 2" strokeLinecap="round" strokeLinejoin="round" /></svg>
}

function normalizedProfile(profile: UserProfile): UserProfile {
  const displayName = profile.displayName?.trim() || null
  const email = profile.email?.trim() || null
  return { ...profile, displayName, email }
}

function validateProfile(
  profile: UserProfile,
): PreferencesDialogProps["profileFieldErrors"] {
  const errors: PreferencesDialogProps["profileFieldErrors"] = {}
  if (
    profile.displayName &&
    ([...profile.displayName].length > 80 || [...profile.displayName].some((value) => /\p{Cc}/u.test(value)))
  ) {
    errors.displayName = "INVALID"
  }
  if (profile.email && !/^\S+@\S+\.\S+$/u.test(profile.email)) {
    errors.email = "INVALID"
  }
  return errors
}
