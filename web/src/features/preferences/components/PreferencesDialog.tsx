import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Stack } from "@astryxdesign/core/Stack"
import { Tab, TabList } from "@astryxdesign/core/TabList"
import { useLingui } from "@lingui/react"
import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"

import type { UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"
import { OpmlTransferPanel } from "../../opml/components/OpmlTransferPanel"

interface PreferencesDialogProps {
  isOpen: boolean
  initialTab?: PreferencesTab
  preferences: UserPreferences
  isSaving: boolean
  error: PreferencesControllerError | null
  csrfToken: string
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onSave: (draft: UserPreferences) => Promise<boolean>
  onSubscriptionsChanged: () => Promise<void> | void
}

export type PreferencesTab = "appearance" | "subscriptions"

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
  const errorCopy = props.error
    ? {
        title: i18n._(
          props.error === "LOAD" ? "preferences.loadError" : "preferences.saveError",
        ),
        description: i18n._(
          props.error === "LOAD"
            ? "preferences.loadErrorDescription"
            : "preferences.saveErrorDescription",
        ),
      }
    : null

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
                <Tab
                  value="subscriptions"
                  label={i18n._("preferences.tabSubscriptions")}
                />
              </TabList>
            </div>
            <div className="reader-preferences-panel">
              {activeTab === "appearance" ? (
                <form id="reader-preferences-form" onSubmit={submit}>
                  <Stack gap={5}>
                    {errorCopy ? (
                      <Banner
                        status="error"
                        title={errorCopy.title}
                        description={errorCopy.description}
                      />
                    ) : null}
                    <PreferenceField
                      label={i18n._("preferences.appearance")}
                      description={i18n._("preferences.appearanceDescription")}
                    >
                      <SegmentedControl
                        label={i18n._("preferences.appearance")}
                        value={draft.themeMode}
                        onChange={(value) =>
                          update({ themeMode: value as UserPreferences["themeMode"] })
                        }
                        layout="fill"
                        isDisabled={props.isSaving}
                      >
                        <SegmentedControlItem
                          value="SYSTEM"
                          label={i18n._("preferences.themeSystem")}
                        />
                        <SegmentedControlItem
                          value="LIGHT"
                          label={i18n._("preferences.themeLight")}
                        />
                        <SegmentedControlItem
                          value="DARK"
                          label={i18n._("preferences.themeDark")}
                        />
                      </SegmentedControl>
                    </PreferenceField>
                    <PreferenceField
                      label={i18n._("preferences.language")}
                      description={i18n._("preferences.languageDescription")}
                    >
                      <SegmentedControl
                        label={i18n._("preferences.language")}
                        value={draft.locale}
                        onChange={(value) =>
                          update({ locale: value as UserPreferences["locale"] })
                        }
                        layout="fill"
                        isDisabled={props.isSaving}
                      >
                        <SegmentedControlItem value="zh-CN" label="中文" />
                        <SegmentedControlItem value="en" label="English" />
                      </SegmentedControl>
                    </PreferenceField>
                    <PreferenceField
                      label={i18n._("preferences.density")}
                      description={i18n._("preferences.densityDescription")}
                    >
                      <SegmentedControl
                        label={i18n._("preferences.density")}
                        value={draft.layoutDensity}
                        onChange={(value) =>
                          update({
                            layoutDensity: value as UserPreferences["layoutDensity"],
                          })
                        }
                        layout="fill"
                        isDisabled={props.isSaving}
                      >
                        <SegmentedControlItem
                          value="COMPACT"
                          label={i18n._("preferences.densityCompact")}
                        />
                        <SegmentedControlItem
                          value="BALANCED"
                          label={i18n._("preferences.densityBalanced")}
                        />
                        <SegmentedControlItem
                          value="SPACIOUS"
                          label={i18n._("preferences.densitySpacious")}
                        />
                      </SegmentedControl>
                    </PreferenceField>
                    <PreferenceField
                      label={i18n._("preferences.readingSize")}
                      description={i18n._("preferences.readingSizeDescription")}
                    >
                      <SegmentedControl
                        label={i18n._("preferences.readingSize")}
                        value={String(draft.readingFontScale)}
                        onChange={(value) =>
                          update({ readingFontScale: Number(value) })
                        }
                        layout="fill"
                        isDisabled={props.isSaving}
                      >
                        {[90, 100, 110, 120].map((scale) => (
                          <SegmentedControlItem
                            key={scale}
                            value={String(scale)}
                            label={`${scale}%`}
                          />
                        ))}
                      </SegmentedControl>
                    </PreferenceField>
                  </Stack>
                </form>
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

function PreferenceField({
  label,
  description,
  children,
}: {
  label: string
  description: string
  children: ReactNode
}) {
  return (
    <Stack gap={2} className="reader-preference-field">
      <div>
        <div className="reader-preference-label">{label}</div>
        <div className="reader-preference-description">{description}</div>
      </div>
      {children}
    </Stack>
  )
}
